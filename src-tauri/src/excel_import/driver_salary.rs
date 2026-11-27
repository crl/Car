use super::{
    amount, cell, err, fingerprint, header_indices, Detail, ExcelImporter, ImportRun,
    PreparedSheet, Result, SheetProject, WholeSheetRun, Workbook,
};
use calamine::{Data, Range, Reader};
use rusqlite::{params, OptionalExtension, Transaction};
use rust_decimal::{prelude::ToPrimitive, Decimal, RoundingStrategy};
use std::collections::{BTreeMap, BTreeSet};

pub(super) const SALARY: &str = "司机工资";
pub(super) const FEE: &str = "外包服务费";
const HEADERS: &[&str] = &["车牌号", SALARY];

fn find_header(range: &Range<Data>, project: &str) -> Option<(usize, Vec<usize>)> {
    range.rows().enumerate().find_map(|(index, row)| {
        header_indices(row, &["车牌号", project]).map(|columns| (index, columns))
    })
}

pub(super) fn discover(workbook: &mut Workbook) -> Result<Vec<SheetProject>> {
    let mut layouts = Vec::new();
    for name in workbook.sheet_names() {
        let values = workbook.worksheet_range(&name).map_err(err)?;
        layouts.push((
            name,
            find_header(&values, SALARY).is_some(),
            find_header(&values, FEE).is_some(),
        ));
    }
    let plain_salary = layouts.iter().any(|(_, salary, fee)| *salary && !fee);
    let mut choices = Vec::new();
    for (sheet, salary, fee) in layouts {
        if salary && (!plain_salary || !fee) {
            choices.push(SheetProject {
                sheet: sheet.clone(),
                category: SALARY.into(),
            });
        }
        if fee {
            choices.push(SheetProject {
                sheet,
                category: FEE.into(),
            });
        }
    }
    Ok(choices)
}

pub(super) fn fee_run(file_stem: &str) -> WholeSheetRun {
    WholeSheetRun {
        file_stem: file_stem.to_owned(),
        source_table: "excel_service_fee_sources",
        prepare: prepare_fees,
    }
}

// An older client may already have imported the identical salary column from
// a sheet that the current layout offers only as a fee source.
pub(super) fn prior_mixed_salaries(
    tx: &Transaction<'_>,
    stem: &str,
    choices: &[SheetProject],
) -> Result<BTreeSet<String>> {
    let salary_sheets: BTreeSet<_> = choices
        .iter()
        .filter(|s| s.category == SALARY)
        .map(|s| &s.sheet)
        .collect();
    let mut fingerprints = BTreeSet::new();
    for source in choices
        .iter()
        .filter(|s| s.category == FEE && !salary_sheets.contains(&s.sheet))
    {
        let previous:Option<String>=tx.query_row("SELECT fingerprint FROM excel_salary_sources WHERE file_stem=? AND sheet=? LIMIT 1",
            params![stem,source.sheet],|row| row.get(0)).optional().map_err(err)?;
        if let Some(hash) = previous {
            fingerprints.insert(hash);
        }
    }
    Ok(fingerprints)
}

pub(super) struct DriverSalaryImporter;

impl ExcelImporter for DriverSalaryImporter {
    fn requires_party(&self) -> bool {
        false
    }

    fn kind(&self) -> &'static str {
        "支出"
    }

    fn headers(&self) -> &'static [&'static str] {
        HEADERS
    }

    fn find_header(&self, range: &Range<Data>) -> Option<(usize, Vec<usize>)> {
        find_header(range, SALARY)
    }

    fn start(&self, file_stem: &str) -> Box<dyn ImportRun> {
        Box::new(WholeSheetRun {
            file_stem: file_stem.to_owned(),
            source_table: "excel_salary_sources",
            prepare: prepare_sheet,
        })
    }
}

fn is_explicit_zero(value: &Data) -> bool {
    match value {
        Data::Int(n) => *n == 0,
        Data::Float(n) => *n == 0.0,
        Data::String(text) => {
            let text = text.trim();
            // Check the significand too: a nonzero fraction can underflow to 0 as f64.
            let significand = text.split(['e', 'E']).next().unwrap_or("");
            text.parse::<f64>() == Ok(0.0) && !significand.bytes().any(|c| matches!(c, b'1'..=b'9'))
        }
        _ => false,
    }
}

fn prepare_sheet(
    workbook: &mut Workbook,
    sheet: &str,
    errors: &mut Vec<String>,
) -> Result<PreparedSheet> {
    let mut rows = Vec::new();
    let count = read_rows(workbook, sheet, SALARY, errors, |row, plate, value| {
        rows.push(Detail {
            row,
            merged_rows: Vec::new(),
            excluded: false,
            start: String::new(),
            end: String::new(),
            plate,
            cents: amount(Some(value))?,
        });
        Ok(())
    })?;
    Ok(PreparedSheet::new(fingerprint(&rows)?, count, rows))
}

fn prepare_fees(
    workbook: &mut Workbook,
    sheet: &str,
    errors: &mut Vec<String>,
) -> Result<PreparedSheet> {
    let mut amounts = BTreeMap::<String, (Decimal, Vec<usize>)>::new();
    let mut details = Vec::new();
    let count = read_rows(workbook, sheet, FEE, errors, |row, plate, value| {
        let text = cell(value);
        let fee = Decimal::from_str_exact(&text)
            .or_else(|_| Decimal::from_scientific(&text))
            .map_err(|_| "外包服务费金额无效")?;
        if fee <= Decimal::ZERO || fee > Decimal::from(999999999) {
            return Err("外包服务费金额无效或超出上限".into());
        }
        details.push((plate.clone(), fee.normalize().to_string()));
        let (sum, rows) = amounts.entry(plate).or_default();
        *sum = sum.checked_add(fee).ok_or("外包服务费合计超出上限")?;
        rows.push(row);
        Ok(())
    })?;
    let mut rows = Vec::new();
    for (plate, (sum, merged_rows)) in amounts {
        let rounded = sum.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero);
        if rounded.is_zero() {
            continue;
        }
        if rounded > Decimal::from(999999999) {
            errors.push(format!(
                "「{sheet}」车牌「{plate}」合并后的外包服务费超过单笔上限"
            ));
            continue;
        }
        let cents = (rounded * Decimal::from(100))
            .to_i64()
            .ok_or("外包服务费金额无效")?;
        rows.push(Detail {
            row: merged_rows[0],
            merged_rows,
            plate,
            cents,
            excluded: false,
            start: String::new(),
            end: String::new(),
        });
    }
    rows.sort_by_key(|row| row.row);
    Ok(PreparedSheet::new(fingerprint(&details)?, count, rows))
}

fn read_rows(
    workbook: &mut Workbook,
    sheet: &str,
    project: &str,
    errors: &mut Vec<String>,
    mut accept: impl FnMut(usize, String, &Data) -> Result<()>,
) -> Result<usize> {
    let values = workbook.worksheet_range(sheet).map_err(err)?;
    let (header, columns) = find_header(&values, project)
        .ok_or_else(|| format!("子表「{sheet}」需要同一行表头：车牌号、{project}"))?;
    let (row_offset, col_offset) = values.start().unwrap();
    let plate_col = col_offset + columns[0] as u32;
    let salary_col = col_offset + columns[1] as u32;
    let formulas = workbook.worksheet_formula(sheet).map_err(err)?;
    // A formula without a saved result can be beyond the range of cached values.
    let end_row = values
        .end()
        .unwrap()
        .0
        .max(formulas.end().map_or(0, |pos| pos.0));
    let data_rows = row_offset + header as u32 + 1..=end_row;
    let mut count = 0;
    for row in data_rows {
        let plate = values
            .get_value((row, plate_col))
            .map(cell)
            .unwrap_or_default()
            .to_uppercase();
        let salary = values.get_value((row, salary_col)).unwrap_or(&Data::Empty);
        let formula = formulas
            .get_value((row, salary_col))
            .map(|s| s.trim())
            .unwrap_or("");
        let blank = cell(salary).is_empty();
        if plate.is_empty() && blank && formula.is_empty() {
            continue;
        }
        let is_footer = plate.is_empty() && row == end_row && {
            let formula = formula
                .trim_start_matches('=')
                .split_whitespace()
                .collect::<String>()
                .to_ascii_uppercase();
            (formula.starts_with("SUM(") && formula.ends_with(')'))
                || (formula.starts_with("ROUND(SUM(") && formula.ends_with("),2)"))
        };
        if is_footer {
            continue;
        }
        if is_explicit_zero(salary) {
            continue;
        }
        count += 1;
        let row_number = row as usize + 1;
        let result = if blank && !formula.is_empty() {
            Err(format!(
                "{project}公式缺少已保存的计算结果，请在 Excel 中重新计算并保存"
            ))
        } else if blank {
            Err(format!("{project}为空"))
        } else if plate.is_empty() {
            Err("车牌号缺失".into())
        } else {
            accept(row_number, plate, salary)
        };
        if let Err(error) = result {
            errors.push(format!("「{sheet}」第 {row_number} 行：{error}"));
        }
    }
    Ok(count)
}
