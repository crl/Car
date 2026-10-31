//! XLSX export of the caller's already filtered and ordered ledger rows.
use rust_xlsxwriter::{Color, Format, Workbook, XlsxError};
use serde_json::Value;
use std::path::Path;

pub fn write_workbook(path: &Path, rows: &[Value]) -> Result<(), String> {
    write_inner(path, rows).map_err(|e| e.to_string())
}

fn text<'a>(row: &'a Value, key: &str) -> &'a str {
    row.get(key).and_then(Value::as_str).unwrap_or("")
}

fn write_inner(path: &Path, rows: &[Value]) -> Result<(), XlsxError> {
    // Header, spacer, and three summary rows must fit Excel's row limit.
    if rows.len() > 1_048_571 {
        return Err(XlsxError::ParameterError("Too many export rows".into()));
    }
    // Sum integers before converting to spreadsheet numbers. i128 also prevents
    // overflow when many individually valid i64 amounts are accumulated.
    let mut income = 0i128;
    let mut expense = 0i128;
    let mut amounts = Vec::with_capacity(rows.len());
    for row in rows {
        let cents = row.get("cents").and_then(Value::as_i64).ok_or_else(|| {
            XlsxError::ParameterError("Entry cents must be an integer in the i64 range".into())
        })?;
        if text(row, "kind") == "收入" {
            income += i128::from(cents);
        } else {
            expense += i128::from(cents);
        }
        amounts.push(cents);
    }
    let mut book = Workbook::new();
    let sheet = book.add_worksheet();
    sheet.set_name("收支明细")?;
    sheet.set_freeze_panes(1, 0)?;
    sheet.autofilter(0, 0, rows.len() as u32, 9)?;
    let header = Format::new()
        .set_bold()
        .set_font_color(Color::White)
        .set_background_color(Color::RGB(0x1E3A6B));
    let positive = Format::new()
        .set_num_format("#,##0.00")
        .set_font_color(Color::RGB(0x148365))
        .set_background_color(Color::RGB(0xEAF7F1));
    let negative = Format::new()
        .set_num_format("#,##0.00")
        .set_font_color(Color::RGB(0xBF6630))
        .set_background_color(Color::RGB(0xFFF2E9));
    let neutral = Format::new().set_num_format("#,##0.00");
    for (col, width) in [14, 20, 10, 14, 18, 18, 40, 40, 20, 55]
        .into_iter()
        .enumerate()
    {
        sheet.set_column_width(col as u16, f64::from(width))?;
    }
    for (col, label) in [
        "日期",
        "车牌",
        "收支",
        "项目",
        "收入（元）",
        "支出（元）",
        "起点",
        "终点",
        "往来方",
        "备注",
    ]
    .into_iter()
    .enumerate()
    {
        sheet.write_string_with_format(0, col as u16, label, &header)?;
    }
    for (index, (row, cents)) in rows.iter().zip(amounts).enumerate() {
        let r = index as u32 + 1;
        let fallback = |key| {
            let s = text(row, key);
            if s.is_empty() {
                "未记录"
            } else {
                s
            }
        };
        let plate = text(row, "plate");
        for (col, value) in [
            (0, text(row, "date")),
            (
                1,
                if plate.is_empty() {
                    text(row, "name")
                } else {
                    plate
                },
            ),
            (2, text(row, "kind")),
            (3, text(row, "category")),
            (6, fallback("start_address")),
            (7, fallback("end_address")),
            (8, text(row, "party")),
            (9, text(row, "note")),
        ] {
            // Explicit strings keep formula-looking input and URLs as literal text.
            sheet.write_string(r, col, value)?;
        }
        let is_income = text(row, "kind") == "收入";
        sheet.write_number_with_format(
            r,
            if is_income { 4 } else { 5 },
            cents as f64 / 100.0,
            if is_income { &positive } else { &negative },
        )?;
    }
    let summary = rows.len() as u32 + 2;
    let balance = income - expense;
    for (offset, label, col, cents, format) in [
        (0, "合计收入", 4, income, &positive),
        (1, "合计支出", 5, expense, &negative),
        (
            2,
            "收支差额",
            4,
            balance,
            if balance > 0 {
                &positive
            } else if balance < 0 {
                &negative
            } else {
                &neutral
            },
        ),
    ] {
        sheet.write_string(summary + offset, 0, label)?;
        sheet.write_number_with_format(summary + offset, col, cents as f64 / 100.0, format)?;
    }
    book.save(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Read;

    fn part(path: &Path, name: &str) -> String {
        let mut archive = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
        let mut xml = String::new();
        archive
            .by_name(name)
            .unwrap()
            .read_to_string(&mut xml)
            .unwrap();
        xml
    }

    #[test]
    fn layout_literals_totals_and_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ledger.xlsx");
        write_workbook(
            &path,
            &[
                json!({"date":"second","kind":"收入","cents":10,"name":"=1+1","note":"@SUM(A1)"}),
                json!({"date":"first","kind":"支出","cents":30,"plate":"+plate"}),
                json!({"kind":"收入","cents":20}),
            ],
        )
        .unwrap();
        let xml = part(&path, "xl/worksheets/sheet1.xml");
        let strings = part(&path, "xl/sharedStrings.xml");
        assert!(xml.contains("ySplit=\"1\""));
        assert!(xml.contains("autoFilter ref=\"A1:J4\""));
        assert!(!xml.contains("<f"));
        assert!(strings.find("second").unwrap() < strings.find("first").unwrap());
        for literal in ["=1+1", "@SUM(A1)", "+plate", "未记录"] {
            assert!(strings.contains(literal));
        }
        assert!(xml.contains("r=\"E6\" s=\"2\"><v>0.3</v>"));
        assert!(xml.contains("r=\"F7\" s=\"3\"><v>0.3</v>"));
        assert!(xml.contains("r=\"E8\" s=\"4\"><v>0</v>"));
        let styles = part(&path, "xl/styles.xml");
        for color in ["FF1E3A6B", "FF148365", "FFEAF7F1", "FFBF6630", "FFFFF2E9"] {
            assert!(styles.contains(color));
        }
    }

    #[test]
    fn empty_and_invalid_input() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.xlsx");
        write_workbook(&path, &[]).unwrap();
        assert!(part(&path, "xl/worksheets/sheet1.xml").contains("autoFilter ref=\"A1:J1\""));
        assert!(write_workbook(&path, &[json!({"cents":1.5})]).is_err());
    }
}
