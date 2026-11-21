use super::{cents, err, Ledger, Result};
use calamine::{open_workbook, Data, Reader, Xlsx};
use rusqlite::{params, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs::File,
    hash::{DefaultHasher, Hash, Hasher},
    io::BufReader,
    path::Path,
};

pub fn workbook(path: &Path) -> Result<Xlsx<BufReader<File>>> {
    if path
        .extension()
        .and_then(|s| s.to_str())
        .is_none_or(|s| !s.eq_ignore_ascii_case("xlsx"))
    {
        return Err("请选择 .xlsx 文件".into());
    }
    if path.metadata().map_err(err)?.len() > 20 * 1024 * 1024 {
        return Err("Excel 文件不能超过 20 MB".into());
    }
    open_workbook(path).map_err(|e| format!("无法读取 Excel：{e}"))
}

#[derive(Deserialize)]
pub struct ImportOptions {
    pub sheet: String,
    date: String,
    party: String,
    kind: String,
    category: String,
}

#[derive(Default)]
struct Group {
    first_row: usize,
    cents: i64,
    sources: Vec<(String, String, String)>,
}

fn cell(value: &Data) -> String {
    value.to_string().trim().to_owned()
}

impl Ledger {
    pub fn import_excel(
        &self,
        path: &Path,
        options: ImportOptions,
        expected: Option<Value>,
    ) -> Result<Value> {
        let ImportOptions {
            sheet,
            date,
            party,
            kind,
            category,
        } = options;
        let party = party.trim();
        let date_value = chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d");
        if date.starts_with("0000")
            || !date_value.is_ok_and(|d| d.to_string() == date)
            || party.is_empty()
        {
            return Err("请填写有效的记账日期和往来方".into());
        }
        let range = workbook(path)?.worksheet_range(&sheet).map_err(err)?;
        let start_row = range.start().map_or(0, |(row, _)| row);
        let mut rows = range.rows();
        let header = rows.next().ok_or("子表为空")?;
        let columns = [
            "VIN",
            "始发地",
            "目的地",
            "上游指令单号",
            "对账金额",
            "卡车号",
        ];
        let indices = columns.map(|name| header.iter().position(|c| cell(c) == name));
        if indices.iter().any(Option::is_none) {
            return Err(format!("子表「{sheet}」需要表头：{}", columns.join("、")));
        }
        let indices = indices.map(Option::unwrap);
        let tx = self.conn.unchecked_transaction().map_err(err)?;
        if !matches!(kind.as_str(), "收入" | "支出") || category.is_empty() {
            return Err("请选择收支方向和项目".into());
        }
        let active: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM categories WHERE kind=? AND name=? AND active=1)",
                params![kind, category],
                |r| r.get(0),
            )
            .map_err(err)?;
        if !active {
            return Err("所选收支项目已停用或不存在，请重新打开导入窗口".into());
        }
        let mut groups: BTreeMap<_, Group> = BTreeMap::new();
        let mut plate_order = HashMap::new();
        let mut errors = Vec::new();
        let mut addresses = BTreeMap::new();
        let mut seen = HashMap::new();
        let mut details = 0;
        let mut duplicates = 0;
        let mut source_total = 0;
        let mut duplicate_total = 0;
        for (offset, row) in rows.enumerate() {
            let [vin, start, end, order, amount, plate] =
                indices.map(|i| row.get(i).map_or(String::new(), cell));
            if [&vin, &start, &end, &order, &amount, &plate]
                .iter()
                .all(|s| s.is_empty())
            {
                continue;
            }
            details += 1;
            let row_number = start_row as usize + offset + 2;
            let plate = plate.to_uppercase();
            plate_order.entry(plate.clone()).or_insert(row_number);
            let mut add_error =
                |message: &str| errors.push(format!("「{sheet}」第 {row_number} 行：{message}"));
            if [&vin, &start, &end, &order, &plate]
                .iter()
                .any(|s| s.is_empty())
            {
                add_error("VIN、路线、指令单号或卡车号缺失");
                continue;
            }
            let amount = match row.get(indices[4]) {
                Some(Data::Float(n))
                    if n.is_finite() && (n * 100.0 - (n * 100.0).round()).abs() <= 1e-5 =>
                {
                    cents(&json!(format!("{n:.2}")))
                }
                _ => cents(&json!(amount)),
            };
            let amount = match amount {
                Ok(amount) => amount,
                Err(error) => {
                    add_error(&error);
                    continue;
                }
            };
            source_total += amount;
            let vin = vin.to_uppercase();
            let key = (vin.clone(), order.clone());
            let fingerprint = json!([plate, start, end, amount]).to_string();
            let saved: Option<String> = tx
                .query_row(
                    "SELECT fingerprint FROM excel_import_sources WHERE vin=? AND order_number=?",
                    params![vin, order],
                    |r| r.get(0),
                )
                .optional()
                .map_err(err)?;
            if let Some(previous) = saved.as_ref().or_else(|| seen.get(&key)) {
                if previous == &fingerprint {
                    duplicates += 1;
                    duplicate_total += amount;
                } else {
                    add_error("相同 VIN 和指令单号的车牌、路线或金额不一致");
                }
                continue;
            }
            seen.insert(key, fingerprint.clone());
            let mut lookup = tx
                .prepare_cached("SELECT id FROM cars WHERE upper(trim(plate))=? LIMIT 2")
                .map_err(err)?;
            let cars = lookup
                .query_map([&plate], |r| r.get::<_, i64>(0))
                .map_err(err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(err)?;
            let car = match cars.as_slice() {
                [] => None,
                [car] => Some(*car),
                _ => {
                    add_error(&format!("卡车号「{plate}」匹配到多辆车，请先核对车辆档案"));
                    continue;
                }
            };
            for name in [&start, &end] {
                if !addresses.contains_key(name) {
                    let existing: Option<i64> = tx
                        .query_row(
                            "SELECT id FROM addresses WHERE active=1 AND name=?",
                            [name],
                            |r| r.get(0),
                        )
                        .optional()
                        .map_err(err)?;
                    addresses.insert(name.clone(), existing);
                }
            }
            let group = groups
                .entry((car, plate, start, end))
                .or_insert_with(|| Group {
                    first_row: row_number,
                    ..Group::default()
                });
            group.cents += amount;
            group.sources.push((vin, order, fingerprint));
        }
        let mut groups: Vec<_> = groups.into_iter().collect();
        groups.sort_by_key(|((_, plate, ..), group)| (plate_order[plate], group.first_row));
        let mut entries = Vec::new();
        let mut new_plates = BTreeSet::new();
        let mut digest = DefaultHasher::new();
        let new_addresses: Vec<_> = addresses
            .iter()
            .filter_map(|(name, id)| id.is_none().then_some(name))
            .collect();
        for ((car, plate, start, end), group) in &groups {
            if car.is_none() {
                new_plates.insert(plate);
            }
            (start, addresses[start], end, addresses[end], &group.sources).hash(&mut digest);
            if group.cents > 99999999900 {
                errors.push(format!("卡车号「{plate}」合并后的金额超过单笔上限"));
            }
            entries.push(json!({
                "car":car,"plate":plate,"start":start,
                "end":end,"cents":group.cents,
                "sources":group.sources.len()
            }));
        }
        let total: i64 = groups.iter().map(|(_, group)| group.cents).sum();
        let mut new_plates: Vec<_> = new_plates.into_iter().collect();
        new_plates.sort_by_key(|plate| plate_order[*plate]);
        let preview = json!({"sheet":sheet,"date":date,"party":party,"kind":kind,"category":category,"details":details,
            "duplicates":duplicates,"errors":errors,"entries":entries,
            "total":total,"source_total":source_total,"duplicate_total":duplicate_total,
            "new_addresses":new_addresses,"new_plates":new_plates,
            "revision":format!("{:016x}", digest.finish())});
        let Some(expected) = expected else {
            return Ok(preview);
        };
        if preview != expected || !errors.is_empty() || groups.is_empty() {
            return Err("导入内容已变化或有未解决的问题，请重新预览".into());
        }
        for (name, id) in &mut addresses {
            if id.is_none() {
                self.save("addresses", json!({"name":name}))?;
                *id = Some(tx.last_insert_rowid());
            }
        }
        let mut new_cars = HashMap::new();
        for plate in new_plates {
            tx.execute(
                "INSERT INTO cars(name,plate,vin,note) VALUES('',?,'','')",
                [plate],
            )
            .map_err(err)?;
            new_cars.insert(plate, tx.last_insert_rowid());
        }
        for ((car, plate, start, end), group) in &groups {
            let car = car.unwrap_or_else(|| new_cars[plate]);
            self.save("entries", json!({"car_id":car,"date":date,"party":party,"kind":kind,
                "category":category,"amount":format!("{}.{:02}", group.cents / 100, group.cents % 100),
                "start_address_id":addresses[start],"end_address_id":addresses[end]}))?;
            let entry_id = tx.last_insert_rowid();
            // Keep the workbook labels even when a reused address has extra details.
            tx.execute(
                "UPDATE entries SET start_address=?,end_address=? WHERE id=?",
                params![start, end, entry_id],
            )
            .map_err(err)?;
            for (vin, order, fingerprint) in &group.sources {
                tx.execute("INSERT INTO excel_import_sources(vin,order_number,fingerprint,entry_id) VALUES(?,?,?,?)",
                    params![vin,order,fingerprint,entry_id]).map_err(err)?;
            }
        }
        tx.commit().map_err(err)?;
        Ok(json!({"count":groups.len(),"new_cars":new_cars.len()}))
    }
}
