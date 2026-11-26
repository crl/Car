use super::{
    cell, err, field, fingerprint, read_table, ExcelImporter, Group, ImportRun, PreparedSheet,
    RawDetail, Result, SheetImport, Workbook,
};
use rusqlite::{params, OptionalExtension, Transaction};
use std::collections::HashMap;

const HEADERS: &[&str] = &["装车地", "目的地", "公司付倒板费", "车牌号", "派车单编号"];
const FEE_COLUMNS: &[&str] = &["预付油款", "出车款", "撬装", "万金油", "公司付倒板费"];

pub(super) struct TransferFeeImporter;

impl ExcelImporter for TransferFeeImporter {
    fn kind(&self) -> &'static str {
        "支出"
    }

    fn headers(&self) -> &'static [&'static str] {
        HEADERS
    }

    fn start(&self, _file_stem: &str) -> Box<dyn ImportRun> {
        Box::new(TransferFeeRun::default())
    }
}

#[derive(Default)]
struct TransferFeeRun {
    // Each accepted dispatch belongs to the ledger entry for its row's vehicle and route.
    seen: HashMap<String, String>,
    dispatches: HashMap<String, HashMap<usize, String>>,
}

impl ImportRun for TransferFeeRun {
    fn read_sheet(
        &mut self,
        tx: &Transaction<'_>,
        workbook: &mut Workbook,
        sheet: &str,
        errors: &mut Vec<String>,
    ) -> Result<PreparedSheet> {
        let (range, columns) = read_table(workbook, sheet, HEADERS)?;
        let first_row = range.start().unwrap().0 as usize;
        let header = range.rows().next().unwrap();
        let last = range
            .rows()
            .rposition(|row| row.iter().any(|v| !cell(v).is_empty()));
        let dispatches = self.dispatches.entry(sheet.to_owned()).or_default();
        let mut lookup = tx
            .prepare_cached(
                "SELECT fingerprint FROM excel_transfer_fee_sources WHERE dispatch_number=?",
            )
            .map_err(err)?;
        let mut details = Vec::new();
        let mut rows = Vec::new();
        let mut count = 0;
        let mut duplicates = 0;
        let mut duplicate_total = 0;
        for (index, row) in range.rows().enumerate().skip(1) {
            if row.iter().all(|v| cell(v).is_empty()) {
                continue;
            }
            if Some(index) == last
                && row.iter().zip(header).all(|(v, label)| {
                    cell(v).is_empty() || FEE_COLUMNS.contains(&cell(label).as_str())
                })
            {
                continue;
            }
            count += 1;
            let row_number = first_row + index + 1;
            let dispatch = field(row, columns[4]);
            let raw = RawDetail::read(row, &columns);
            if dispatch.is_empty() || !raw.has_route_and_plate() {
                errors.push(format!(
                    "「{sheet}」第 {row_number} 行：派车单编号、车牌号、装车地或目的地缺失"
                ));
                continue;
            }
            let mut detail = match raw.into_detail(row_number) {
                Ok(detail) => detail,
                Err(error) => {
                    errors.push(format!("「{sheet}」第 {row_number} 行：{error}"));
                    continue;
                }
            };
            let signature = fingerprint(&detail)?;
            details.push((dispatch.clone(), signature.clone()));
            let saved: Option<String> = lookup
                .query_row([&dispatch], |r| r.get(0))
                .optional()
                .map_err(err)?;
            if let Some(previous) = self.seen.get(&dispatch).or(saved.as_ref()) {
                detail.excluded = true;
                if previous == &signature {
                    duplicates += 1;
                    duplicate_total += detail.cents;
                } else {
                    errors.push(format!("「{sheet}」第 {row_number} 行：派车单编号「{dispatch}」重复，车牌、线路或金额不一致"));
                }
            } else {
                self.seen.insert(dispatch.clone(), signature);
                dispatches.insert(row_number, dispatch);
            }
            rows.push(detail);
        }
        let mut prepared = PreparedSheet::new(fingerprint(&details)?, count, rows);
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
        let mut insert = tx.prepare_cached(
            "INSERT INTO excel_transfer_fee_sources(dispatch_number,fingerprint,entry_id) VALUES(?,?,?)",
        ).map_err(err)?;
        let dispatches = &self.dispatches[&sheet.name];
        for row in &group.source_rows {
            let dispatch = &dispatches[row];
            insert
                .execute(params![dispatch, self.seen[dispatch], entry_id])
                .map_err(err)?;
        }
        Ok(())
    }
}
