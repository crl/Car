use super::{cents, err, Ledger, Result};
use calamine::{open_workbook, Data, Reader, Xlsx};
use rusqlite::{params, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs::File,
    io::BufReader,
    path::Path,
};

#[cfg(test)]
#[path = "excel_import/batch_tests.rs"]
mod batch_tests;

type Workbook = Xlsx<BufReader<File>>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Format {
    Freight,
    Pickup,
}

impl Format {
    fn headers(self) -> &'static [&'static str] {
        match self {
            Self::Freight => &[
                "始发地",
                "目的地",
                "对账金额",
                "卡车号",
                "VIN",
                "上游指令单号",
            ],
            Self::Pickup => &["始发地", "目的地", "提车费", "卡车号"],
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Freight => "运费",
            Self::Pickup => "提车费",
        }
    }

    fn kind(self) -> &'static str {
        match self {
            Self::Freight => "收入",
            Self::Pickup => "支出",
        }
    }
}

#[derive(Deserialize)]
pub struct ImportOptions {
    #[serde(default)]
    sheet: String,
    #[serde(default)]
    sheets: Vec<String>,
    date: String,
    party: String,
    kind: String,
    category: String,
}

#[derive(Serialize)]
struct Detail {
    #[serde(skip)]
    row: usize,
    vin: String,
    order: String,
    start: String,
    end: String,
    plate: String,
    cents: i64,
}

struct Group {
    first_row: usize,
    car: Option<i64>,
    plate: String,
    start: String,
    end: String,
    cents: i64,
    details: usize,
    freight_sources: Vec<(String, String, String)>,
}

struct SheetImport {
    name: String,
    fingerprint: String,
    details: usize,
    source_total: i64,
    duplicates: usize,
    duplicate_total: i64,
    groups: Vec<Group>,
}

impl SheetImport {
    fn total(&self) -> i64 {
        self.groups.iter().map(|group| group.cents).sum()
    }

    fn summary(&self) -> Value {
        json!({"sheet":self.name,"details":self.details,"source_total":self.source_total,
            "duplicates":self.duplicates,"duplicate_total":self.duplicate_total,
            "count":self.groups.len(),"total":self.total()})
    }
}

struct FreightRow {
    fingerprint: String,
    sheet: String,
    row: usize,
}

#[derive(Default)]
struct Batch {
    addresses: BTreeMap<String, Option<i64>>,
    freight_seen: HashMap<(String, String), FreightRow>,
    errors: Vec<String>,
}

fn cell(value: &Data) -> String {
    value.to_string().trim().to_owned()
}

fn header_indices(header: &[Data], format: Format) -> Option<Vec<usize>> {
    let cells: Vec<_> = header.iter().map(cell).collect();
    format
        .headers()
        .iter()
        .map(|name| cells.iter().position(|value| value == name))
        .collect()
}

fn fingerprint(value: &impl Serialize) -> Result<String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).map_err(err)?)
    ))
}

fn amount(value: Option<&Data>) -> Result<i64> {
    match value {
        Some(Data::Float(n))
            if n.is_finite() && (n * 100.0 - (n * 100.0).round()).abs() <= 1e-5 =>
        {
            cents(&json!(format!("{n:.2}")))
        }
        _ => cents(&json!(value.map_or_else(String::new, cell))),
    }
}

fn read_details(
    workbook: &mut Workbook,
    sheet: &str,
    format: Format,
    errors: &mut Vec<String>,
) -> Result<(usize, Vec<Detail>)> {
    let range = workbook.worksheet_range(sheet).map_err(err)?;
    let (first_row, first_col) = range
        .start()
        .ok_or_else(|| format!("子表「{sheet}」为空"))?;
    let header = range.rows().next().unwrap();
    let columns = header_indices(header, format)
        .ok_or_else(|| format!("子表「{sheet}」需要表头：{}", format.headers().join("、")))?;
    let (start_col, end_col, amount_col, plate_col) =
        (columns[0], columns[1], columns[2], columns[3]);
    let vin_col = columns
        .get(4)
        .copied()
        .or_else(|| header.iter().position(|value| cell(value) == "VIN"));
    let order_col = columns.get(5).copied();
    let pickup_total_row = if format == Format::Pickup {
        let last = range
            .rows()
            .rposition(|row| row.iter().any(|value| !cell(value).is_empty()));
        let formulas = workbook.worksheet_formula(sheet).map_err(err)?;
        last.filter(|&row| {
            row > 0
                && range.rows().nth(row).is_some_and(|cells| {
                    cells
                        .iter()
                        .enumerate()
                        .all(|(col, value)| col == amount_col || cell(value).is_empty())
                })
                && formulas
                    .get_value((first_row + row as u32, first_col + amount_col as u32))
                    .is_some_and(|formula| {
                        formula
                            .trim()
                            .trim_start_matches('=')
                            .trim_start()
                            .to_ascii_uppercase()
                            .starts_with("SUM(")
                    })
        })
    } else {
        None
    };

    let mut details = Vec::new();
    let mut count = 0;
    for (index, row) in range.rows().enumerate().skip(1) {
        if pickup_total_row == Some(index) {
            continue;
        }
        let field = |column| row.get(column).map_or_else(String::new, cell);
        let start = field(start_col);
        let end = field(end_col);
        let plate = field(plate_col).to_uppercase();
        let vin = vin_col.map_or_else(String::new, field).to_uppercase();
        let order = order_col.map_or_else(String::new, field);
        if [&start, &end, &plate, &vin, &order]
            .iter()
            .all(|value| value.is_empty())
            && field(amount_col).is_empty()
        {
            continue;
        }
        count += 1;
        let row_number = first_row as usize + index + 1;
        if start.is_empty()
            || end.is_empty()
            || plate.is_empty()
            || (format == Format::Freight && (vin.is_empty() || order.is_empty()))
        {
            let message = match format {
                Format::Freight => "VIN、路线、指令单号或卡车号缺失",
                Format::Pickup => "始发地、目的地或卡车号缺失",
            };
            errors.push(format!("「{sheet}」第 {row_number} 行：{message}"));
            continue;
        }
        match amount(row.get(amount_col)) {
            Ok(cents) => details.push(Detail {
                row: row_number,
                vin,
                order,
                start,
                end,
                plate,
                cents,
            }),
            Err(error) => errors.push(format!("「{sheet}」第 {row_number} 行：{error}")),
        }
    }
    Ok((count, details))
}

impl Batch {
    fn prepare_sheet(
        &mut self,
        tx: &Transaction<'_>,
        workbook: &mut Workbook,
        file_stem: &str,
        name: String,
        format: Format,
    ) -> Result<SheetImport> {
        let (count, details) = read_details(workbook, &name, format, &mut self.errors)?;
        let mut sheet = SheetImport {
            name,
            fingerprint: fingerprint(&details)?,
            details: count,
            source_total: details.iter().map(|detail| detail.cents).sum(),
            duplicates: 0,
            duplicate_total: 0,
            groups: Vec::new(),
        };
        if format == Format::Pickup {
            let existing: Option<String> = tx.query_row(
                "SELECT fingerprint FROM excel_pickup_sources WHERE file_stem=? AND sheet=? LIMIT 1",
                params![file_stem, sheet.name],
                |row| row.get(0),
            ).optional().map_err(err)?;
            if let Some(previous) = existing {
                if previous == sheet.fingerprint {
                    sheet.duplicates = details.len();
                    sheet.duplicate_total = sheet.source_total;
                } else {
                    self.errors.push(format!(
                        "子表「{}」已导入，当前内容与已入账版本不同，请先核对账目",
                        sheet.name
                    ));
                }
                return Ok(sheet);
            }
        }

        let mut groups = BTreeMap::new();
        let mut plate_order = HashMap::new();
        for detail in &details {
            plate_order.entry(&detail.plate).or_insert(detail.row);
            let freight_source = if format == Format::Freight {
                let key = (detail.vin.clone(), detail.order.clone());
                let source =
                    json!([detail.plate, detail.start, detail.end, detail.cents]).to_string();
                let saved: Option<String> = tx.query_row(
                    "SELECT fingerprint FROM excel_import_sources WHERE vin=? AND order_number=?",
                    params![detail.vin, detail.order],
                    |row| row.get(0),
                ).optional().map_err(err)?;
                let first_row = self.freight_seen.get(&key);
                let previous = saved
                    .as_deref()
                    .or_else(|| first_row.map(|row| row.fingerprint.as_str()));
                if let Some(previous) = previous {
                    if previous == source {
                        sheet.duplicates += 1;
                        sheet.duplicate_total += detail.cents;
                    } else {
                        let origin = first_row.map_or_else(
                            || "已入账明细".to_owned(),
                            |row| format!("「{}」第 {} 行", row.sheet, row.row),
                        );
                        self.errors.push(format!(
                            "「{}」第 {} 行：与{origin}的 VIN 和指令单号相同，车牌、路线或金额不一致",
                            sheet.name, detail.row
                        ));
                    }
                    continue;
                }
                let (vin, order) = key.clone();
                self.freight_seen.insert(
                    key,
                    FreightRow {
                        fingerprint: source.clone(),
                        sheet: sheet.name.clone(),
                        row: detail.row,
                    },
                );
                Some((vin, order, source))
            } else {
                None
            };

            let mut lookup = tx
                .prepare_cached("SELECT id FROM cars WHERE upper(trim(plate))=? LIMIT 2")
                .map_err(err)?;
            let cars = lookup
                .query_map([&detail.plate], |row| row.get(0))
                .map_err(err)?
                .collect::<rusqlite::Result<Vec<i64>>>()
                .map_err(err)?;
            let car = match cars.as_slice() {
                [] => None,
                [car] => Some(*car),
                _ => {
                    self.errors.push(format!(
                        "「{}」第 {} 行：卡车号「{}」匹配到多辆车，请先核对车辆档案",
                        sheet.name, detail.row, detail.plate
                    ));
                    continue;
                }
            };
            for name in [&detail.start, &detail.end] {
                if !self.addresses.contains_key(name) {
                    let existing = tx
                        .query_row(
                            "SELECT id FROM addresses WHERE active=1 AND name=?",
                            [name],
                            |row| row.get(0),
                        )
                        .optional()
                        .map_err(err)?;
                    self.addresses.insert(name.clone(), existing);
                }
            }

            let group = groups
                .entry((&detail.plate, &detail.start, &detail.end))
                .or_insert_with(|| Group {
                    first_row: detail.row,
                    car,
                    plate: detail.plate.clone(),
                    start: detail.start.clone(),
                    end: detail.end.clone(),
                    cents: 0,
                    details: 0,
                    freight_sources: Vec::new(),
                });
            group.cents += detail.cents;
            group.details += 1;
            group.freight_sources.extend(freight_source);
        }
        sheet.groups = groups.into_values().collect();
        sheet
            .groups
            .sort_by_key(|group| (plate_order[&group.plate], group.first_row));
        for group in &sheet.groups {
            if group.cents > 99999999900 {
                self.errors.push(format!(
                    "子表「{}」卡车号「{}」合并后的金额超过单笔上限",
                    sheet.name, group.plate
                ));
            }
        }
        Ok(sheet)
    }
}

impl Ledger {
    fn import_workbook<'a>(&self, path: &'a Path) -> Result<(&'a str, Format, Workbook)> {
        if path
            .extension()
            .and_then(|s| s.to_str())
            .is_none_or(|s| !s.eq_ignore_ascii_case("xlsx"))
        {
            return Err("请选择 .xlsx 文件".into());
        }
        let file_stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default();
        let (_, category) = file_stem
            .rsplit_once('_')
            .filter(|(prefix, category)| !prefix.trim().is_empty() && !category.trim().is_empty())
            .ok_or("请将文件命名为 xxx_收支类型.xlsx，例如 11月对账_运费.xlsx")?;
        let format = if category == "提车费" {
            Format::Pickup
        } else {
            Format::Freight
        };
        let active: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM categories WHERE name=?1 AND active=1 AND kind IN ('收入','支出') AND (?2=0 OR kind='支出'))",
            params![category, format == Format::Pickup],
            |row| row.get(0),
        ).map_err(err)?;
        if !active {
            return Err(format!("文件名中的收支类型『{category}』不存在或已停用"));
        }
        if path.metadata().map_err(err)?.len() > 20 * 1024 * 1024 {
            return Err("Excel 文件不能超过 20 MB".into());
        }
        let workbook = open_workbook(path).map_err(|error| format!("无法读取 Excel：{error}"))?;
        Ok((file_stem, format, workbook))
    }

    pub fn excel_import_info(&self, path: &Path) -> Result<Value> {
        let (_, format, mut workbook) = self.import_workbook(path)?;
        let mut sheets = Vec::new();
        for name in workbook.sheet_names() {
            let range = workbook.worksheet_range(&name).map_err(err)?;
            if range
                .rows()
                .next()
                .and_then(|header| header_indices(header, format))
                .is_some()
            {
                sheets.push(name);
            }
        }
        if sheets.is_empty() {
            return Err(format!(
                "此文件没有符合{}格式的子表，需要表头：{}",
                format.label(),
                format.headers().join("、")
            ));
        }
        Ok(json!({
            "path":path,"sheets":sheets,"kind":format.kind(),"default_category":format.label()
        }))
    }

    pub fn import_excel(
        &self,
        path: &Path,
        options: ImportOptions,
        expected: Option<Value>,
    ) -> Result<Value> {
        let ImportOptions {
            sheet,
            mut sheets,
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
        if sheets.is_empty() && !sheet.is_empty() {
            sheets.push(sheet);
        }
        if sheets.is_empty() {
            return Err("请至少选择一张子表".into());
        }
        let (file_stem, format, mut workbook) = self.import_workbook(path)?;
        if kind != format.kind() {
            return Err(format!("{}文件只能导入为{}", format.label(), format.kind()));
        }
        let selected: BTreeSet<_> = sheets.iter().map(String::as_str).collect();
        let ordered: Vec<_> = workbook
            .sheet_names()
            .into_iter()
            .filter(|sheet| selected.contains(sheet.as_str()))
            .collect();
        if ordered.len() != sheets.len() {
            return Err("所选子表不存在或重复，请重新选择文件".into());
        }

        let tx = self.conn.unchecked_transaction().map_err(err)?;
        if category.is_empty() {
            return Err("请选择收支项目".into());
        }
        let active: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM categories WHERE kind=? AND name=? AND active=1)",
                params![kind, category],
                |row| row.get(0),
            )
            .map_err(err)?;
        if !active {
            return Err("所选收支项目已停用或不存在，请重新打开导入窗口".into());
        }

        let mut batch = Batch::default();
        let sheets = ordered
            .into_iter()
            .map(|name| batch.prepare_sheet(&tx, &mut workbook, file_stem, name, format))
            .collect::<Result<Vec<_>>>()?;
        let groups = sheets.iter().flat_map(|sheet| &sheet.groups);
        let mut plates_seen = BTreeSet::new();
        let new_plates: Vec<_> = groups
            .clone()
            .filter(|group| group.car.is_none() && plates_seen.insert(&group.plate))
            .map(|group| &group.plate)
            .collect();
        let new_addresses: Vec<_> = batch
            .addresses
            .iter()
            .filter_map(|(name, id)| id.is_none().then_some(name))
            .collect();
        let entries: Vec<_> =
            sheets
                .iter()
                .flat_map(|sheet| {
                    sheet.groups.iter().map(|group| json!({
                "sheet":sheet.name,"car":group.car,"plate":group.plate,"start":group.start,
                "end":group.end,"cents":group.cents,"sources":group.details
            }))
                })
                .collect();
        let summaries: Vec<_> = sheets.iter().map(SheetImport::summary).collect();
        let sources: Vec<_> = sheets
            .iter()
            .map(|sheet| (&sheet.name, &sheet.fingerprint))
            .collect();
        let preview = json!({
            "sheets":summaries,"sheet":(sheets.len() == 1).then(|| &sheets[0].name),
            "date":date,"party":party,"kind":kind,"category":category,
            "details":sheets.iter().map(|sheet| sheet.details).sum::<usize>(),
            "duplicates":sheets.iter().map(|sheet| sheet.duplicates).sum::<usize>(),
            "errors":batch.errors,"entries":entries,
            "total":sheets.iter().map(SheetImport::total).sum::<i64>(),
            "source_total":sheets.iter().map(|sheet| sheet.source_total).sum::<i64>(),
            "duplicate_total":sheets.iter().map(|sheet| sheet.duplicate_total).sum::<i64>(),
            "new_addresses":new_addresses,"new_plates":new_plates,
            "revision":fingerprint(&(file_stem, sources, &batch.addresses))?
        });
        let Some(expected) = expected else {
            return Ok(preview);
        };
        if preview != expected || !batch.errors.is_empty() || entries.is_empty() {
            return Err("导入内容已变化或有未解决的问题，请重新预览".into());
        }

        for (name, id) in &mut batch.addresses {
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
        for sheet in &sheets {
            for group in &sheet.groups {
                let car = group.car.unwrap_or_else(|| new_cars[&group.plate]);
                self.save("entries", json!({"car_id":car,"date":date,"party":party,"kind":kind,
                    "category":category,"amount":format!("{}.{:02}", group.cents / 100, group.cents % 100),
                    "start_address_id":batch.addresses[&group.start],"end_address_id":batch.addresses[&group.end]}))?;
                let entry_id = tx.last_insert_rowid();
                // Keep the workbook labels even when a reused address has extra details.
                tx.execute(
                    "UPDATE entries SET start_address=?,end_address=? WHERE id=?",
                    params![group.start, group.end, entry_id],
                )
                .map_err(err)?;
                match format {
                    Format::Freight => {
                        for (vin, order, fingerprint) in &group.freight_sources {
                            tx.execute("INSERT INTO excel_import_sources(vin,order_number,fingerprint,entry_id) VALUES(?,?,?,?)",
                                params![vin,order,fingerprint,entry_id]).map_err(err)?;
                        }
                    }
                    Format::Pickup => {
                        tx.execute("INSERT INTO excel_pickup_sources(file_stem,sheet,fingerprint,entry_id) VALUES(?,?,?,?)",
                            params![file_stem,sheet.name,sheet.fingerprint,entry_id]).map_err(err)?;
                    }
                }
            }
        }
        tx.commit().map_err(err)?;
        Ok(json!({"count":entries.len(),"new_cars":new_cars.len()}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> ImportOptions {
        ImportOptions {
            sheet: "明细".into(),
            sheets: Vec::new(),
            date: "2026-11-24".into(),
            party: "华通".into(),
            kind: "收入".into(),
            category: "运费".into(),
        }
    }

    #[test]
    fn invalid_names_are_rejected_before_reading_the_workbook() {
        let folder = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(folder.path().join("ledger.db")).unwrap();
        ledger
            .conn
            .execute("UPDATE categories SET active=0 WHERE name='通行费'", [])
            .unwrap();

        for (name, expected) in [
            ("11月对账.xlsx", "xxx_收支类型.xlsx"),
            ("_运费.xlsx", "xxx_收支类型.xlsx"),
            ("11月对账_.xlsx", "xxx_收支类型.xlsx"),
            ("11月对账_收入.xlsx", "『收入』不存在或已停用"),
            ("11月对账_通行费.xlsx", "『通行费』不存在或已停用"),
        ] {
            let path = folder.path().join(name);
            std::fs::write(&path, "invalid workbook").unwrap();
            let error = ledger.import_excel(&path, options(), None).unwrap_err();
            assert!(error.contains(expected), "{name}: {error}");
        }
    }

    #[test]
    fn enabled_income_and_expense_names_allow_preview() {
        let folder = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(folder.path().join("ledger.db")).unwrap();
        let mut workbook = rust_xlsxwriter::Workbook::new();
        let sheet = workbook.add_worksheet().set_name("明细").unwrap();
        sheet
            .write_row(
                0,
                0,
                [
                    "VIN",
                    "始发地",
                    "目的地",
                    "上游指令单号",
                    "对账金额",
                    "卡车号",
                ],
            )
            .unwrap()
            .write_row(1, 0, ["VIN01", "上海", "杭州", "单号01", "120", "沪A12345"])
            .unwrap();

        for name in ["11月_对账_运费.xlsx", "11月对账_油费.xlsx"] {
            let path = folder.path().join(name);
            workbook.save(&path).unwrap();
            let preview = ledger.import_excel(&path, options(), None).unwrap();
            assert_eq!(preview["details"], 1);
            assert_eq!(preview["total"], 12000);
            assert_eq!(preview["category"], "运费");
        }
    }
}
