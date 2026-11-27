use super::{
    amount, cell, err, fingerprint, header_indices, Detail, ExcelImporter, ImportRun,
    PreparedSheet, Result, WholeSheetRun, Workbook,
};
use calamine::{Data, Range, Reader};

const HEADERS: &[&str] = &["车牌号", "司机工资"];

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
        range
            .rows()
            .enumerate()
            .find_map(|(index, row)| header_indices(row, HEADERS).map(|columns| (index, columns)))
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
    let values = workbook.worksheet_range(sheet).map_err(err)?;
    let (header, columns) = DriverSalaryImporter
        .find_header(&values)
        .ok_or_else(|| format!("子表「{sheet}」需要同一行表头：车牌号、司机工资"))?;
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
    let mut rows = Vec::new();
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
        let is_footer = plate.is_empty()
            && row == end_row
            && formula.ends_with(')')
            && formula
                .trim_start_matches('=')
                .trim_start()
                .get(..4)
                .is_some_and(|name| name.eq_ignore_ascii_case("SUM("));
        if is_footer {
            continue;
        }
        if is_explicit_zero(salary) {
            continue;
        }
        count += 1;
        let row_number = row as usize + 1;
        let result = if blank && !formula.is_empty() {
            Err("司机工资公式缺少已保存的计算结果，请在 Excel 中重新计算并保存".into())
        } else if blank {
            Err("司机工资为空".into())
        } else if plate.is_empty() {
            Err("车牌号缺失".into())
        } else {
            amount(Some(salary))
        };
        match result {
            Ok(cents) => rows.push(Detail {
                row: row_number,
                excluded: false,
                plate,
                cents,
                start: String::new(),
                end: String::new(),
            }),
            Err(error) => errors.push(format!("「{sheet}」第 {row_number} 行：{error}")),
        }
    }
    Ok(PreparedSheet::new(fingerprint(&rows)?, count, rows))
}
