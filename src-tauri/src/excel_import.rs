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

#[path = "excel_import/driver_salary.rs"]
mod driver_salary;
#[path = "excel_import/freight.rs"]
mod freight;
#[path = "excel_import/pickup.rs"]
mod pickup;
#[path = "excel_import/transfer_fee.rs"]
mod transfer_fee;

type Workbook = Xlsx<BufReader<File>>;

trait ExcelImporter: Sync + Send {
    fn kind(&self) -> &'static str;
    fn headers(&self) -> &'static [&'static str];

    fn requires_party(&self) -> bool {
        true
    }

    fn find_header(&self, range: &Range<Data>) -> Option<(usize, Vec<usize>)> {
        header_indices(range.rows().next()?, self.headers()).map(|columns| (0, columns))
    }

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
            .register("司机工资", driver_salary::DriverSalaryImporter)
            .expect("unique driver salary importer");
        registry
            .register("运费", freight::FreightImporter)
            .expect("unique freight importer");
        registry
            .register("提车费", pickup::PickupImporter)
            .expect("unique pickup importer");
        registry
            .register("倒板费", transfer_fee::TransferFeeImporter)
            .expect("unique transfer fee importer");
        registry
    })
}

#[derive(Deserialize)]
pub struct ImportOptions {
    #[serde(default)]
    sheet: String,
    #[serde(default)]
    sheets: Vec<String>,
    #[serde(default)]
    sources: Option<Vec<SheetProject>>,
    date: String,
    #[serde(default)]
    party: String,
    kind: String,
    #[serde(default)]
    category: String,
}

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
struct SheetProject {
    sheet: String,
    category: String,
}

#[derive(Serialize)]
struct Detail {
    #[serde(skip)]
    row: usize,
    #[serde(skip)]
    merged_rows: Vec<usize>,
    #[serde(skip)]
    excluded: bool,
    start: String,
    end: String,
    plate: String,
    cents: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    brand: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    party: Option<String>,
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
    warnings: Vec<String>,
}

impl PreparedSheet {
    fn skip_imported_rows(&mut self) {
        self.duplicates = self
            .rows
            .iter()
            .map(|row| row.merged_rows.len().max(1))
            .sum();
        self.duplicate_total = self.source_total;
        self.rows.clear();
    }

    fn new(fingerprint: String, details: usize, rows: Vec<Detail>) -> Self {
        Self {
            fingerprint,
            details,
            source_total: rows.iter().map(|row| row.cents).sum(),
            duplicates: 0,
            duplicate_total: 0,
            rows,
            warnings: Vec::new(),
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
    brand: String,
    party: Option<String>,
}

impl Group {
    fn merge_note(&self) -> String {
        let count = self.source_rows.len();
        if count > 1 {
            format!("由{count}笔合并")
        } else {
            String::new()
        }
    }
}

// One source fingerprint per sheet, linked to every ledger entry it produced.
struct WholeSheetRun {
    file_stem: String,
    source_table: &'static str,
    prepare: fn(&mut Workbook, &str, &mut Vec<String>) -> Result<PreparedSheet>,
}

impl ImportRun for WholeSheetRun {
    fn read_sheet(
        &mut self,
        tx: &Transaction<'_>,
        workbook: &mut Workbook,
        sheet: &str,
        errors: &mut Vec<String>,
    ) -> Result<PreparedSheet> {
        let mut prepared = (self.prepare)(workbook, sheet, errors)?;
        let sql = format!(
            "SELECT fingerprint FROM {} WHERE file_stem=? AND sheet=? LIMIT 1",
            self.source_table
        );
        let existing: Option<String> = tx
            .query_row(&sql, params![self.file_stem, sheet], |row| row.get(0))
            .optional()
            .map_err(err)?;
        if let Some(previous) = existing {
            if previous == prepared.fingerprint {
                prepared.skip_imported_rows();
            } else {
                errors.push(format!(
                    "子表「{sheet}」已导入，当前内容与已入账版本不同，请先核对账目"
                ));
                prepared.rows.clear();
            }
        }
        Ok(prepared)
    }

    fn record_sources(
        &self,
        tx: &Transaction<'_>,
        sheet: &SheetImport,
        _group: &Group,
        entry_id: i64,
    ) -> Result<()> {
        let sql = format!(
            "INSERT INTO {}(file_stem,sheet,fingerprint,entry_id) VALUES(?,?,?,?)",
            self.source_table
        );
        tx.execute(
            &sql,
            params![self.file_stem, sheet.name, sheet.fingerprint, entry_id],
        )
        .map_err(err)?;
        Ok(())
    }
}

struct SheetImport {
    name: String,
    category: String,
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
        json!({"sheet":self.name,"category":self.category,"details":self.details,"source_total":self.source_total,
            "duplicates":self.duplicates,"duplicate_total":self.duplicate_total,
            "count":self.groups.len(),"total":self.total()})
    }
}

#[derive(Default)]
struct Batch {
    warnings: Vec<String>,
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
            merged_rows: Vec::new(),
            excluded: false,
            brand: String::new(),
            party: None,
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
        source: SheetProject,
        prepared: PreparedSheet,
    ) -> Result<SheetImport> {
        let PreparedSheet {
            fingerprint,
            details,
            source_total,
            duplicates,
            duplicate_total,
            rows,
            warnings,
        } = prepared;
        self.warnings.extend(warnings);
        let mut sheet = SheetImport {
            name: source.sheet,
            category: source.category,
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
                .entry((&detail.plate, &detail.start, &detail.end, &detail.brand, &detail.party))
                .or_insert_with(|| Group {
                    first_row: detail.row,
                    car,
                    plate: detail.plate.clone(),
                    start: detail.start.clone(),
                    end: detail.end.clone(),
                    cents: 0,
                    source_rows: Vec::new(),
                    brand: detail.brand.clone(),
                    party: detail.party.clone(),
                });
            group.cents += detail.cents;
            if detail.merged_rows.is_empty() {
                group.source_rows.push(detail.row);
            } else {
                group.source_rows.extend_from_slice(&detail.merged_rows);
            }
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

fn category_active(conn: &rusqlite::Connection, kind: &str, name: &str) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM categories WHERE kind=? AND name=? AND active=1)",
        params![kind, name],
        |row| row.get(0),
    )
    .map_err(err)
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
        let mut importer = self.resolve(ledger, stem)?;
        if path.metadata().map_err(err)?.len() > 20 * 1024 * 1024 {
            return Err("Excel 文件不能超过 20 MB".into());
        }
        let mut workbook: Workbook = open_workbook(path).map_err(|error| format!("无法读取 Excel：{error}"))?;
        if importer.name == "运费" && workbook.sheet_names().iter().any(|sheet| {
            workbook.worksheet_range(sheet).is_ok_and(|range| freight::MODERN.find_header(&range).is_some())
        }) {
            importer.handler = &freight::MODERN;
        }
        Ok((stem, importer, workbook))
    }

    fn inspect(&self, ledger: &Ledger, path: &Path) -> Result<Value> {
        let (_, importer, mut workbook) = self.open(ledger, path)?;
        let headers = importer.handler.headers();
        if importer.name == driver_salary::SALARY {
            let choices = driver_salary::discover(&mut workbook)?;
            let mixed = choices
                .iter()
                .any(|source| source.category == driver_salary::FEE);
            if mixed {
                let fee_active = category_active(&ledger.conn, "支出", driver_salary::FEE)?;
                let sources: Vec<_> = choices.iter().map(|s| json!({
                    "sheet": s.sheet, "category": s.category, "enabled": s.category != driver_salary::FEE || fee_active
                })).collect();
                return Ok(
                    json!({"path":path, "sources":sources, "kind":"支出","default_category":importer.name,"requires_party":false}),
                );
            }
            if choices.is_empty() {
                return Err(
                    "此文件没有符合司机工资格式的子表，需要车牌号和司机工资或外包服务费表头".into(),
                );
            }
            return Ok(
                json!({"path":path,"sheets":choices.iter().map(|s| &s.sheet).collect::<Vec<_>>(),
                "kind":"支出","default_category":importer.name,"requires_party":false}),
            );
        }
        let mut sheets = Vec::new();
        for name in workbook.sheet_names() {
            let range = workbook.worksheet_range(&name).map_err(err)?;
            if importer.handler.find_header(&range).is_some() {
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
            "path":path,"sheets":sheets,"kind":importer.handler.kind(),"default_category":importer.name,
            "requires_party":importer.handler.requires_party()
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
            sources,
            date,
            party,
            kind,
            category,
        } = options;
        let party = party.trim();
        let date_value = chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d");
        if date.starts_with("0000") || !date_value.is_ok_and(|d| d.to_string() == date) {
            return Err("请填写有效的记账日期".into());
        }
        if sheets.is_empty() && !sheet.is_empty() {
            sheets.push(sheet);
        }
        if sheets.is_empty() && sources.as_ref().is_none_or(Vec::is_empty) {
            return Err("请至少选择一张子表".into());
        }
        let (file_stem, importer, mut workbook) = self.open(ledger, path)?;
        if importer.handler.requires_party() && party.is_empty() {
            return Err("请填写往来方".into());
        }
        if kind != importer.handler.kind() {
            return Err(format!(
                "{}文件只能导入为{}",
                importer.name,
                importer.handler.kind()
            ));
        }
        let salary_choices = if importer.name == driver_salary::SALARY {
            driver_salary::discover(&mut workbook)?
        } else {
            Vec::new()
        };
        let mixed = salary_choices
            .iter()
            .any(|s| s.category == driver_salary::FEE);
        let tx = ledger.conn.unchecked_transaction().map_err(err)?;
        let old_salary_fingerprints =
            driver_salary::prior_mixed_salaries(&tx, file_stem, &salary_choices)?;
        let ordered: Vec<SheetProject> = if mixed {
            let selected = sources
                .filter(|s| !s.is_empty() && sheets.is_empty())
                .ok_or("工资或服务费来源已变化，请重新选择文件")?;
            let keys: BTreeSet<_> = selected.iter().collect();
            let ordered: Vec<_> = salary_choices
                .into_iter()
                .filter(|s| keys.contains(s))
                .collect();
            if ordered.len() != selected.len() {
                return Err("所选子表或收支项目已变化或重复，请重新选择文件".into());
            }
            ordered
        } else {
            if sources.is_some() {
                return Err("导入来源已变化，请重新选择文件".into());
            }
            if category.is_empty() {
                return Err("请选择收支项目".into());
            }
            let selected: BTreeSet<_> = sheets.iter().collect();
            let ordered: Vec<_> = workbook
                .sheet_names()
                .into_iter()
                .filter(|s| selected.contains(s))
                .map(|sheet| SheetProject {
                    sheet,
                    category: category.clone(),
                })
                .collect();
            if ordered.len() != sheets.len() {
                return Err("所选子表不存在或重复，请重新选择文件".into());
            }
            ordered
        };
        let categories: BTreeSet<_> = ordered.iter().map(|s| s.category.as_str()).collect();
        for name in &categories {
            if !category_active(&tx, &kind, name)? {
                return Err(format!(
                    "所选收支项目已停用或不存在：{name}，请重新打开导入窗口"
                ));
            }
        }
        let category = categories.into_iter().collect::<Vec<_>>().join("、");

        let mut run = importer.handler.start(file_stem);
        let mut fee_run = driver_salary::fee_run(file_stem);
        let mut batch = Batch::default();
        let mut address_names = HashMap::new();
        let sheets = ordered
            .into_iter()
            .map(|source| {
                let handler: &mut dyn ImportRun = if mixed && source.category == driver_salary::FEE
                {
                    &mut fee_run
                } else {
                    run.as_mut()
                };
                let mut prepared =
                    handler.read_sheet(&tx, &mut workbook, &source.sheet, &mut batch.errors)?;
                if mixed
                    && source.category == driver_salary::SALARY
                    && !prepared.rows.is_empty()
                    && old_salary_fingerprints.contains(&prepared.fingerprint)
                {
                    prepared.skip_imported_rows();
                }
                let mut resolve_address = tx.prepare_cached(
                    "SELECT name FROM addresses WHERE active=1 AND
                     (trim(name)=?1 COLLATE NOCASE OR EXISTS(SELECT 1 FROM json_each(aliases)
                      WHERE trim(value)=?1 COLLATE NOCASE))
                     ORDER BY (name=?1) DESC, (trim(name)=?1 COLLATE NOCASE) DESC, id LIMIT 1"
                ).map_err(err)?;
                for row in &mut prepared.rows {
                    if row.excluded {
                        continue;
                    }
                    for name in [&mut row.start, &mut row.end] {
                        if !name.is_empty() {
                            let key = name.trim().to_ascii_lowercase();
                            if let std::collections::hash_map::Entry::Vacant(slot) = address_names.entry(key.clone()) {
                                let canonical = resolve_address.query_row([name.as_str()],
                                    |row| row.get::<_, String>(0)).optional().map_err(err)?;
                                slot.insert(canonical.unwrap_or_else(|| name.clone()));
                            }
                            name.clone_from(&address_names[&key]);
                        }
                    }
                }
                batch.group_sheet(&tx, source, prepared)
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
                        "sheet": sheet.name, "category": sheet.category, "car": group.car, "plate": group.plate,
                        "start": group.start, "end": group.end, "cents": group.cents,
                        "sources": group.source_rows.len(), "note": group.merge_note(),
                        "brand": group.brand, "party": group.party.as_deref().unwrap_or(party)
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
            "errors":batch.errors,"warnings":batch.warnings,"entries":entries,
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
                        "car_id": car, "date": date, "party": group.party.as_deref().unwrap_or(party), "kind": kind,
                        "brand": group.brand,
                        "category": sheet.category,
                        "note": group.merge_note(),
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
                let handler: &dyn ImportRun = if mixed && sheet.category == driver_salary::FEE {
                    &fee_run
                } else {
                    run.as_ref()
                };
                handler.record_sources(&tx, sheet, group, entry_id)?;
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
            sources: None,
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
