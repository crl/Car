use super::{cents, err, Ledger, Result};
use calamine::{open_workbook, Data, Range, Reader, Xlsx};
use rusqlite::{params, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs::File,
    io::BufReader,
    path::Path,
    sync::OnceLock,
};

#[cfg(test)]
#[path = "excel_import/batch_tests.rs"]
mod batch_tests;

#[path = "excel_import/freight.rs"]
mod freight;
#[path = "excel_import/pickup.rs"]
mod pickup;

type Workbook = Xlsx<BufReader<File>>;

trait ExcelImporter: Sync + Send {
    fn kind(&self) -> &'static str;
    fn headers(&self) -> &'static [&'static str];

    fn matches_filename(&self, stem: &str, category: &str) -> bool {
        stem.strip_suffix(category)
            .and_then(|prefix| prefix.strip_suffix('_'))
            .is_some_and(|prefix| !prefix.trim().is_empty())
    }

    fn start(&self, file_stem: &str) -> Box<dyn ImportRun>;
}

trait ImportRun {
    fn read_sheet(
        &mut self,
        tx: &Transaction<'_>,
        workbook: &mut Workbook,
        sheet: &str,
        errors: &mut Vec<String>,
    ) -> Result<PreparedSheet>;

    fn record_sources(
        &self,
        tx: &Transaction<'_>,
        sheet: &SheetImport,
        group: &Group,
        entry_id: i64,
    ) -> Result<()>;
}

#[derive(Default)]
struct ImportRegistry {
    handlers: BTreeMap<&'static str, Box<dyn ExcelImporter>>,
}

struct RegisteredImporter<'a> {
    name: &'static str,
    handler: &'a dyn ExcelImporter,
}

fn builtin_importers() -> &'static ImportRegistry {
    static REGISTRY: OnceLock<ImportRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let mut registry = ImportRegistry::default();
        registry
            .register("运费", freight::FreightImporter)
            .expect("unique freight importer");
        registry
            .register("提车费", pickup::PickupImporter)
            .expect("unique pickup importer");
        registry
    })
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
    #[serde(skip)]
    excluded: bool,
    start: String,
    end: String,
    plate: String,
    cents: i64,
}

// These field names and their order are part of the existing workbook fingerprints.
#[derive(Serialize)]
struct LegacyDetail {
    vin: String,
    order: String,
    #[serde(flatten)]
    detail: Detail,
}

struct PreparedSheet {
    fingerprint: String,
    details: usize,
    source_total: i64,
    duplicates: usize,
    duplicate_total: i64,
    rows: Vec<Detail>,
}

impl PreparedSheet {
    fn new(fingerprint: String, details: usize, rows: Vec<Detail>) -> Self {
        Self {
            fingerprint,
            details,
            source_total: rows.iter().map(|row| row.cents).sum(),
            duplicates: 0,
            duplicate_total: 0,
            rows,
        }
    }
}

struct Group {
    first_row: usize,
    car: Option<i64>,
    plate: String,
    start: String,
    end: String,
    cents: i64,
    source_rows: Vec<usize>,
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

#[derive(Default)]
struct Batch {
    addresses: BTreeMap<String, Option<i64>>,
    errors: Vec<String>,
}

fn cell(value: &Data) -> String {
    value.to_string().trim().to_owned()
}

fn header_indices(header: &[Data], headers: &[&str]) -> Option<Vec<usize>> {
    let cells: Vec<_> = header.iter().map(cell).collect();
    headers
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

fn read_table(
    workbook: &mut Workbook,
    sheet: &str,
    headers: &[&str],
) -> Result<(Range<Data>, Vec<usize>)> {
    let range = workbook.worksheet_range(sheet).map_err(err)?;
    let header = range
        .rows()
        .next()
        .ok_or_else(|| format!("子表「{sheet}」为空"))?;
    let columns = header_indices(header, headers)
        .ok_or_else(|| format!("子表「{sheet}」需要表头：{}", headers.join("、")))?;
    Ok((range, columns))
}

fn field(row: &[Data], column: usize) -> String {
    row.get(column).map_or_else(String::new, cell)
}

struct RawDetail<'a> {
    start: String,
    end: String,
    plate: String,
    amount: Option<&'a Data>,
}

impl<'a> RawDetail<'a> {
    fn read(row: &'a [Data], columns: &[usize]) -> Self {
        Self {
            start: field(row, columns[0]),
            end: field(row, columns[1]),
            amount: row.get(columns[2]),
            plate: field(row, columns[3]).to_uppercase(),
        }
    }

    fn is_empty(&self) -> bool {
        self.start.is_empty()
            && self.end.is_empty()
            && self.plate.is_empty()
            && self.amount.is_none_or(|value| cell(value).is_empty())
    }

    fn has_route_and_plate(&self) -> bool {
        !self.start.is_empty() && !self.end.is_empty() && !self.plate.is_empty()
    }

    fn into_detail(self, row: usize) -> Result<Detail> {
        Ok(Detail {
            row,
            excluded: false,
            start: self.start,
            end: self.end,
            plate: self.plate,
            cents: amount(self.amount)?,
        })
    }
}

impl Batch {
    fn group_sheet(
        &mut self,
        tx: &Transaction<'_>,
        name: String,
        prepared: PreparedSheet,
    ) -> Result<SheetImport> {
        let PreparedSheet {
            fingerprint,
            details,
            source_total,
            duplicates,
            duplicate_total,
            rows,
        } = prepared;
        let mut sheet = SheetImport {
            name,
            fingerprint,
            details,
            source_total,
            duplicates,
            duplicate_total,
            groups: Vec::new(),
        };
        let mut groups = BTreeMap::new();
        let mut plate_order = HashMap::new();
        for detail in &rows {
            plate_order.entry(&detail.plate).or_insert(detail.row);
            if detail.excluded {
                continue;
            }

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
            for name in [&detail.start, &detail.end]
                .into_iter()
                .filter(|name| !name.is_empty())
            {
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
                    source_rows: Vec::new(),
                });
            group.cents += detail.cents;
            group.source_rows.push(detail.row);
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

fn check_filename_category(ledger: &Ledger, name: &str, kind: Option<&str>) -> Result<()> {
    let active: bool = ledger.conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM categories WHERE name=?1 AND active=1 AND kind IN ('收入','支出') AND (?2 IS NULL OR kind=?2))",
        params![name, kind],
        |row| row.get(0),
    ).map_err(err)?;
    if !active {
        return Err(format!("文件名中的收支类型『{name}』不存在或已停用"));
    }
    Ok(())
}

impl ImportRegistry {
    fn register(
        &mut self,
        name: &'static str,
        handler: impl ExcelImporter + 'static,
    ) -> Result<()> {
        match self.handlers.entry(name) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(Box::new(handler));
                Ok(())
            }
            std::collections::btree_map::Entry::Occupied(_) => {
                Err(format!("收支类型『{name}』已注册"))
            }
        }
    }

    fn resolve(&self, ledger: &Ledger, stem: &str) -> Result<RegisteredImporter<'_>> {
        let matches: Vec<_> = self
            .handlers
            .iter()
            .filter_map(|(&name, handler)| {
                handler
                    .matches_filename(stem, name)
                    .then_some(RegisteredImporter {
                        name,
                        handler: handler.as_ref(),
                    })
            })
            .collect();

        match matches.as_slice() {
            [entry] => {
                check_filename_category(ledger, entry.name, Some(entry.handler.kind()))?;
                Ok(RegisteredImporter {
                    name: entry.name,
                    handler: entry.handler,
                })
            }
            [] => {
                let (_, name) = stem
                    .rsplit_once('_')
                    .filter(|(prefix, name)| !prefix.trim().is_empty() && !name.trim().is_empty())
                    .ok_or("请将文件命名为 xxx_收支类型.xlsx，例如 11月对账_运费.xlsx")?;
                check_filename_category(ledger, name, None)?;
                Err(format!("暂不支持收支类型『{name}』的 Excel 导入"))
            }
            _ => {
                let names = matches
                    .iter()
                    .map(|entry| format!("『{}』", entry.name))
                    .collect::<Vec<_>>();
                Err(format!(
                    "文件名同时匹配收支类型 {}，请修改文件名使其只匹配一种类型",
                    names.join("、")
                ))
            }
        }
    }

    fn open<'a>(
        &self,
        ledger: &Ledger,
        path: &'a Path,
    ) -> Result<(&'a str, RegisteredImporter<'_>, Workbook)> {
        if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_none_or(|extension| !extension.eq_ignore_ascii_case("xlsx"))
        {
            return Err("请选择 .xlsx 文件".into());
        }
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default();
        let importer = self.resolve(ledger, stem)?;
        if path.metadata().map_err(err)?.len() > 20 * 1024 * 1024 {
            return Err("Excel 文件不能超过 20 MB".into());
        }
        let workbook = open_workbook(path).map_err(|error| format!("无法读取 Excel：{error}"))?;
        Ok((stem, importer, workbook))
    }

    fn inspect(&self, ledger: &Ledger, path: &Path) -> Result<Value> {
        let (_, importer, mut workbook) = self.open(ledger, path)?;
        let headers = importer.handler.headers();
        let mut sheets = Vec::new();
        for name in workbook.sheet_names() {
            let range = workbook.worksheet_range(&name).map_err(err)?;
            if range
                .rows()
                .next()
                .and_then(|header| header_indices(header, headers))
                .is_some()
            {
                sheets.push(name);
            }
        }
        if sheets.is_empty() {
            return Err(format!(
                "此文件没有符合{}格式的子表，需要表头：{}",
                importer.name,
                headers.join("、")
            ));
        }
        Ok(json!({
            "path":path,"sheets":sheets,"kind":importer.handler.kind(),"default_category":importer.name
        }))
    }

    fn import(
        &self,
        ledger: &Ledger,
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
        let (file_stem, importer, mut workbook) = self.open(ledger, path)?;
        if kind != importer.handler.kind() {
            return Err(format!(
                "{}文件只能导入为{}",
                importer.name,
                importer.handler.kind()
            ));
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

        let tx = ledger.conn.unchecked_transaction().map_err(err)?;
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

        let mut run = importer.handler.start(file_stem);
        let mut batch = Batch::default();
        let sheets = ordered
            .into_iter()
            .map(|name| {
                let prepared = run.read_sheet(&tx, &mut workbook, &name, &mut batch.errors)?;
                batch.group_sheet(&tx, name, prepared)
            })
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
        let entries: Vec<_> = sheets
            .iter()
            .flat_map(|sheet| {
                sheet.groups.iter().map(|group| {
                    json!({
                        "sheet": sheet.name, "car": group.car, "plate": group.plate,
                        "start": group.start, "end": group.end, "cents": group.cents,
                        "sources": group.source_rows.len()
                    })
                })
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
                ledger.save("addresses", json!({"name":name}))?;
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
        let address_id = |name: &str| batch.addresses.get(name).copied().flatten();
        for sheet in &sheets {
            for group in &sheet.groups {
                let car = group.car.unwrap_or_else(|| new_cars[&group.plate]);
                ledger.save(
                    "entries",
                    json!({
                        "car_id": car, "date": date, "party": party, "kind": kind,
                        "category": category,
                        "amount": format!("{}.{:02}", group.cents / 100, group.cents % 100),
                        "start_address_id": address_id(&group.start),
                        "end_address_id": address_id(&group.end)
                    }),
                )?;
                let entry_id = tx.last_insert_rowid();
                if !group.start.is_empty() || !group.end.is_empty() {
                    // Keep workbook labels even when a reused address has extra details.
                    tx.execute(
                        "UPDATE entries SET start_address=?,end_address=? WHERE id=?",
                        params![group.start, group.end, entry_id],
                    )
                    .map_err(err)?;
                }
                run.record_sources(&tx, sheet, group, entry_id)?;
            }
        }
        tx.commit().map_err(err)?;
        Ok(json!({"count":entries.len(),"new_cars":new_cars.len()}))
    }
}

impl Ledger {
    pub fn excel_import_info(&self, path: &Path) -> Result<Value> {
        builtin_importers().inspect(self, path)
    }

    pub fn import_excel(
        &self,
        path: &Path,
        options: ImportOptions,
        expected: Option<Value>,
    ) -> Result<Value> {
        builtin_importers().import(self, path, options, expected)
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
    fn freight_filenames_allow_prefix_separators_and_case_insensitive_extensions() {
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

        for name in ["11月_对账_运费.xlsx", "11月对账_运费.XLSX"] {
            let path = folder.path().join(name);
            workbook.save(&path).unwrap();
            let preview = ledger.import_excel(&path, options(), None).unwrap();
            assert_eq!(preview["details"], 1);
            assert_eq!(preview["total"], 12000);
            assert_eq!(preview["category"], "运费");
        }
    }
}
