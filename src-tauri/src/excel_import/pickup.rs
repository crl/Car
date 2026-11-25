use super::{
    cell, err, field, fingerprint, read_table, ExcelImporter, Group, ImportRun, LegacyDetail,
    PreparedSheet, RawDetail, Result, SheetImport, Workbook,
};
use calamine::{Data, Range, Reader};
use rusqlite::{params, OptionalExtension, Transaction};

const HEADERS: &[&str] = &["始发地", "目的地", "提车费", "卡车号"];

pub(super) struct PickupImporter;

impl ExcelImporter for PickupImporter {
    fn kind(&self) -> &'static str {
        "支出"
    }

    fn headers(&self) -> &'static [&'static str] {
        HEADERS
    }

    fn matches_filename(&self, stem: &str, category: &str) -> bool {
        stem.contains(&format!("_{category}"))
    }

    fn start(&self, file_stem: &str) -> Box<dyn ImportRun> {
        Box::new(PickupRun {
            file_stem: file_stem.to_owned(),
        })
    }
}

struct PickupRun {
    file_stem: String,
}

fn total_row(
    workbook: &mut Workbook,
    sheet: &str,
    range: &Range<Data>,
    amount_col: usize,
) -> Result<Option<usize>> {
    let (first_row, first_col) = range.start().unwrap();
    let last = range
        .rows()
        .rposition(|row| row.iter().any(|value| !cell(value).is_empty()));
    let formulas = workbook.worksheet_formula(sheet).map_err(err)?;
    Ok(last.filter(|&row| {
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
    }))
}

fn read_details(
    workbook: &mut Workbook,
    sheet: &str,
    errors: &mut Vec<String>,
) -> Result<(usize, Vec<LegacyDetail>)> {
    let (range, columns) = read_table(workbook, sheet, HEADERS)?;
    let first_row = range.start().unwrap().0 as usize;
    let vin_column = range
        .rows()
        .next()
        .unwrap()
        .iter()
        .position(|value| cell(value) == "VIN");
    let total = total_row(workbook, sheet, &range, columns[2])?;
    let mut details = Vec::new();
    let mut count = 0;
    for (index, row) in range.rows().enumerate().skip(1) {
        if total == Some(index) {
            continue;
        }
        let vin = vin_column
            .map_or_else(String::new, |column| field(row, column))
            .to_uppercase();
        let raw = RawDetail::read(row, &columns);
        if raw.is_empty() && vin.is_empty() {
            continue;
        }
        count += 1;
        let row_number = first_row + index + 1;
        if !raw.has_route_and_plate() {
            errors.push(format!(
                "「{sheet}」第 {row_number} 行：始发地、目的地或卡车号缺失"
            ));
            continue;
        }
        match raw.into_detail(row_number) {
            Ok(detail) => details.push(LegacyDetail {
                vin,
                order: String::new(),
                detail,
            }),
            Err(error) => errors.push(format!("「{sheet}」第 {row_number} 行：{error}")),
        }
    }
    Ok((count, details))
}

impl ImportRun for PickupRun {
    fn read_sheet(
        &mut self,
        tx: &Transaction<'_>,
        workbook: &mut Workbook,
        sheet: &str,
        errors: &mut Vec<String>,
    ) -> Result<PreparedSheet> {
        let (count, details) = read_details(workbook, sheet, errors)?;
        let mut prepared = PreparedSheet::new(
            fingerprint(&details)?,
            count,
            details.into_iter().map(|row| row.detail).collect(),
        );
        let existing: Option<String> = tx.query_row(
            "SELECT fingerprint FROM excel_pickup_sources WHERE file_stem=? AND sheet=? LIMIT 1",
            params![self.file_stem, sheet],
            |row| row.get(0),
        ).optional().map_err(err)?;
        if let Some(previous) = existing {
            if previous == prepared.fingerprint {
                prepared.duplicates = prepared.rows.len();
                prepared.duplicate_total = prepared.source_total;
            } else {
                errors.push(format!(
                    "子表「{sheet}」已导入，当前内容与已入账版本不同，请先核对账目"
                ));
            }
            prepared.rows.clear();
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
        tx.execute(
            "INSERT INTO excel_pickup_sources(file_stem,sheet,fingerprint,entry_id) VALUES(?,?,?,?)",
            params![self.file_stem, sheet.name, sheet.fingerprint, entry_id],
        ).map_err(err)?;
        Ok(())
    }
}
