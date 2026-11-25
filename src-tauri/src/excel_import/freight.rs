use super::{
    err, field, fingerprint, read_table, ExcelImporter, Group, ImportRun, LegacyDetail,
    PreparedSheet, RawDetail, Result, SheetImport, Workbook,
};
use rusqlite::{params, OptionalExtension, Transaction};
use serde_json::json;
use std::collections::HashMap;

const HEADERS: &[&str] = &[
    "始发地",
    "目的地",
    "对账金额",
    "卡车号",
    "VIN",
    "上游指令单号",
];

pub(super) struct FreightImporter;

impl ExcelImporter for FreightImporter {
    fn kind(&self) -> &'static str {
        "收入"
    }

    fn headers(&self) -> &'static [&'static str] {
        HEADERS
    }

    fn start(&self, _file_stem: &str) -> Box<dyn ImportRun> {
        Box::new(FreightRun::default())
    }
}

struct FreightSource {
    vin: String,
    order: String,
    fingerprint: String,
}

struct SourceOrigin {
    fingerprint: String,
    sheet: String,
    row: usize,
}

#[derive(Default)]
struct FreightRun {
    seen: HashMap<(String, String), SourceOrigin>,
    sources: HashMap<String, HashMap<usize, FreightSource>>,
}

fn read_details(
    workbook: &mut Workbook,
    sheet: &str,
    errors: &mut Vec<String>,
) -> Result<(usize, Vec<LegacyDetail>)> {
    let (range, columns) = read_table(workbook, sheet, HEADERS)?;
    let first_row = range.start().unwrap().0 as usize;
    let mut details = Vec::new();
    let mut count = 0;
    for (index, row) in range.rows().enumerate().skip(1) {
        let raw = RawDetail::read(row, &columns);
        let vin = field(row, columns[4]).to_uppercase();
        let order = field(row, columns[5]);
        if raw.is_empty() && vin.is_empty() && order.is_empty() {
            continue;
        }
        count += 1;
        let row_number = first_row + index + 1;
        if !raw.has_route_and_plate() || vin.is_empty() || order.is_empty() {
            errors.push(format!(
                "「{sheet}」第 {row_number} 行：VIN、路线、指令单号或卡车号缺失"
            ));
            continue;
        }
        match raw.into_detail(row_number) {
            Ok(detail) => details.push(LegacyDetail { vin, order, detail }),
            Err(error) => errors.push(format!("「{sheet}」第 {row_number} 行：{error}")),
        }
    }
    Ok((count, details))
}

impl ImportRun for FreightRun {
    fn read_sheet(
        &mut self,
        tx: &Transaction<'_>,
        workbook: &mut Workbook,
        sheet: &str,
        errors: &mut Vec<String>,
    ) -> Result<PreparedSheet> {
        let (count, details) = read_details(workbook, sheet, errors)?;
        let sheet_fingerprint = fingerprint(&details)?;
        let source_rows = self.sources.entry(sheet.to_owned()).or_default();
        let mut rows = Vec::with_capacity(details.len());
        let mut duplicates = 0;
        let mut duplicate_total = 0;
        let mut find_source = tx
            .prepare_cached(
                "SELECT fingerprint FROM excel_import_sources WHERE vin=? AND order_number=?",
            )
            .map_err(err)?;

        for LegacyDetail {
            vin,
            order,
            mut detail,
        } in details
        {
            let key = (vin.clone(), order.clone());
            let source = json!([detail.plate, detail.start, detail.end, detail.cents]).to_string();
            let saved: Option<String> = find_source
                .query_row(params![vin, order], |row| row.get(0))
                .optional()
                .map_err(err)?;
            let first_row = self.seen.get(&key);
            let previous = saved
                .as_deref()
                .or_else(|| first_row.map(|row| row.fingerprint.as_str()));
            if let Some(previous) = previous {
                detail.excluded = true;
                if previous == source {
                    duplicates += 1;
                    duplicate_total += detail.cents;
                } else {
                    let origin = first_row.map_or_else(
                        || "已入账明细".to_owned(),
                        |row| format!("「{}」第 {} 行", row.sheet, row.row),
                    );
                    errors.push(format!(
                        "「{sheet}」第 {} 行：与{origin}的 VIN 和指令单号相同，车牌、路线或金额不一致",
                        detail.row
                    ));
                }
            } else {
                self.seen.insert(
                    key,
                    SourceOrigin {
                        fingerprint: source.clone(),
                        sheet: sheet.to_owned(),
                        row: detail.row,
                    },
                );
                source_rows.insert(
                    detail.row,
                    FreightSource {
                        vin,
                        order,
                        fingerprint: source,
                    },
                );
            }
            // Retain excluded rows until grouping to preserve the original order of plates.
            rows.push(detail);
        }
        let mut prepared = PreparedSheet::new(sheet_fingerprint, count, rows);
        prepared.duplicates = duplicates;
        prepared.duplicate_total = duplicate_total;
        Ok(prepared)
    }

    fn record_sources(
        &self,
        tx: &Transaction<'_>,
        sheet: &SheetImport,
        group: &Group,
        entry_id: i64,
    ) -> Result<()> {
        let mut insert_source = tx.prepare_cached(
            "INSERT INTO excel_import_sources(vin,order_number,fingerprint,entry_id) VALUES(?,?,?,?)",
        ).map_err(err)?;
        for row in &group.source_rows {
            let source = self
                .sources
                .get(&sheet.name)
                .and_then(|sources| sources.get(row))
                .ok_or_else(|| format!("「{}」第 {row} 行的运费来源记录缺失", sheet.name))?;
            insert_source
                .execute(params![
                    source.vin,
                    source.order,
                    source.fingerprint,
                    entry_id
                ])
                .map_err(err)?;
        }
        Ok(())
    }
}
