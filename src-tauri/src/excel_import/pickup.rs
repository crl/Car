use super::{
    cell, err, field, fingerprint, read_table, ExcelImporter, ImportRun, LegacyDetail,
    PreparedSheet, RawDetail, Result, WholeSheetRun, Workbook,
};
use calamine::{Data, Range, Reader};

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
        Box::new(WholeSheetRun {
            source_table: "excel_pickup_sources",
            prepare: prepare_sheet,
            file_stem: file_stem.to_owned(),
        })
    }
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

fn prepare_sheet(
    workbook: &mut Workbook,
    sheet: &str,
    errors: &mut Vec<String>,
) -> Result<PreparedSheet> {
    let (count, details) = read_details(workbook, sheet, errors)?;
    Ok(PreparedSheet::new(
        fingerprint(&details)?,
        count,
        details.into_iter().map(|row| row.detail).collect(),
    ))
}
