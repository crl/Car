use super::*;
use rust_xlsxwriter::{Workbook, Worksheet};

fn options(sheets: &[&str], kind: &str, category: &str) -> ImportOptions {
    serde_json::from_value(json!({
        "sheets": sheets, "date": "2026-11-24", "party": "测试往来方",
        "kind": kind, "category": category
    }))
    .unwrap()
}

fn freight_sheet(workbook: &mut Workbook, name: &str, rows: &[(&str, &str)]) {
    let sheet = workbook.add_worksheet().set_name(name).unwrap();
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
        .unwrap();
    for (index, &(vin, amount)) in rows.iter().enumerate() {
        sheet
            .write_row(
                index as u32 + 1,
                0,
                [vin, "上海", "昆山", "单号01", amount, "沪A12345"],
            )
            .unwrap();
    }
}

fn pickup_sheet<'a>(workbook: &'a mut Workbook, name: &str, vin: bool) -> &'a mut Worksheet {
    let sheet = workbook.add_worksheet().set_name(name).unwrap();
    let offset = u16::from(vin);
    if vin {
        sheet.write_string(0, 0, "VIN").unwrap();
        sheet.write_string(1, 0, "VIN03").unwrap();
    }
    sheet
        .write_row(0, offset, ["始发地", "目的地", "提车费", "卡车号"])
        .unwrap()
        .write_row(1, offset, ["上海", "昆山", "50", "沪A12345"])
        .unwrap();
    if !vin {
        sheet
            .write_row(2, offset, ["上海", "昆山", "50", "沪A12345"])
            .unwrap();
    }
    let last = if vin { 2 } else { 3 };
    sheet
        .write_formula(
            last,
            offset + 2,
            if vin { "=SUM(D2:D2)" } else { "=SUM(C2:C3)" },
        )
        .unwrap();
    sheet
}

#[test]
fn freight_batches_group_by_sheet_and_deduplicate_across_sheets_and_history() {
    let folder = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(folder.path().join("ledger.db")).unwrap();
    let path = folder.path().join("本月_运费.xlsx");
    let mut workbook = Workbook::new();
    freight_sheet(&mut workbook, "甲", &[("VIN01", "50")]);
    freight_sheet(&mut workbook, "乙", &[("VIN01", "50"), ("VIN02", "60")]);
    workbook.save(&path).unwrap();
    let options = || options(&["甲", "乙"], "收入", "运费");
    let preview = ledger.import_excel(&path, options(), None).unwrap();
    assert_eq!(preview["details"], 3);
    assert_eq!(preview["duplicates"], 1);
    assert_eq!(preview["total"], 11000);
    assert_eq!(preview["entries"].as_array().unwrap().len(), 2);
    assert_eq!(preview["entries"][0]["sheet"], "甲");
    assert_eq!(preview["entries"][1]["sheet"], "乙");
    let saved = ledger
        .import_excel(&path, options(), Some(preview))
        .unwrap();
    assert_eq!(saved["count"], 2);
    assert_eq!(saved["new_cars"], 1);
    let repeated = ledger.import_excel(&path, options(), None).unwrap();
    assert_eq!(repeated["duplicates"], 3);
    assert_eq!(repeated["total"], 0);
}

#[test]
fn imports_accept_other_projects_in_the_fixed_direction_and_save_the_selection() {
    let folder = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(folder.path().join("ledger.db")).unwrap();
    ledger
        .save("categories", json!({"kind":"收入","name":"其他收入"}))
        .unwrap();
    let mut freight = Workbook::new();
    freight_sheet(&mut freight, "明细", &[("VIN01", "60")]);
    let mut pickup = Workbook::new();
    pickup_sheet(&mut pickup, "明细", false);

    for (workbook, kind, default, alternative, other_kind, other_project, total) in [
        (freight, "收入", "运费", "其他收入", "支出", "油费", 6000),
        (pickup, "支出", "提车费", "油费", "收入", "运费", 10000),
    ] {
        let path = folder.path().join(format!("本月_{default}.xlsx"));
        let mut workbook = workbook;
        workbook.save(&path).unwrap();
        let info = ledger.excel_import_info(&path).unwrap();
        assert_eq!(info["kind"], kind);
        assert_eq!(info["default_category"], default);
        let before = ledger.data().unwrap()["entries"].clone();
        let selection = |category| options(&["明细"], kind, category);
        let preview = ledger
            .import_excel(&path, selection(default), None)
            .unwrap();
        assert!(ledger
            .import_excel(&path, selection(alternative), Some(preview.clone()))
            .is_err());
        assert!(ledger
            .import_excel(&path, selection(other_project), None)
            .is_err());
        assert!(ledger
            .import_excel(&path, options(&["明细"], other_kind, other_project), None)
            .is_err());
        assert!(ledger
            .import_excel(
                &path,
                options(&["明细"], other_kind, other_project),
                Some(preview)
            )
            .is_err());
        let preview = ledger
            .import_excel(&path, selection(alternative), None)
            .unwrap();
        assert_eq!(preview["category"], alternative);
        assert_eq!(preview["total"], total);
        ledger
            .conn
            .execute(
                "UPDATE categories SET active=0 WHERE kind=? AND name=?",
                params![kind, alternative],
            )
            .unwrap();
        assert!(ledger
            .import_excel(&path, selection(alternative), None)
            .is_err());
        assert!(ledger
            .import_excel(&path, selection(alternative), Some(preview))
            .is_err());
        assert_eq!(ledger.data().unwrap()["entries"], before);

        ledger
            .save("categories", json!({"kind":kind,"name":alternative}))
            .unwrap();
        let preview = ledger
            .import_excel(&path, selection(alternative), None)
            .unwrap();
        let result = ledger
            .import_excel(&path, selection(alternative), Some(preview))
            .unwrap();
        assert_eq!(result["count"], 1);
        let entries = ledger.data().unwrap()["entries"].clone();
        assert!(entries
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["kind"] == kind
                && entry["category"] == alternative
                && entry["cents"] == total));
    }
}

#[test]
fn freight_conflicts_and_changes_after_preview_write_nothing() {
    let folder = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(folder.path().join("ledger.db")).unwrap();
    let path = folder.path().join("本月_运费.xlsx");
    let write = |vin| {
        let mut workbook = Workbook::new();
        freight_sheet(&mut workbook, "甲", &[("VIN01", "50")]);
        freight_sheet(&mut workbook, "乙", &[(vin, "60")]);
        workbook.save(&path).unwrap();
    };
    let options = || options(&["甲", "乙"], "收入", "运费");
    write("VIN02");
    let preview = ledger.import_excel(&path, options(), None).unwrap();
    write("VIN01");
    assert!(ledger
        .import_excel(&path, options(), Some(preview))
        .is_err());
    let conflict = ledger.import_excel(&path, options(), None).unwrap();
    assert!(conflict["errors"][0]
        .as_str()
        .unwrap()
        .contains("「乙」第 2 行：与「甲」第 2 行"));
    assert!(ledger
        .import_excel(&path, options(), Some(conflict))
        .is_err());
    assert_eq!(ledger.data().unwrap()["entries"], json!([]));
    assert_eq!(ledger.data().unwrap()["cars"], json!([]));
}

#[test]
fn a_save_error_rolls_back_all_selected_sheets_and_related_records() {
    let folder = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(folder.path().join("ledger.db")).unwrap();
    let path = folder.path().join("本月_运费.xlsx");
    let mut workbook = Workbook::new();
    freight_sheet(&mut workbook, "甲", &[("VIN01", "50")]);
    freight_sheet(&mut workbook, "乙", &[("VIN02", "60")]);
    workbook.save(&path).unwrap();
    let selection = || options(&["甲", "乙"], "收入", "运费");
    let preview = ledger.import_excel(&path, selection(), None).unwrap();
    let before = ledger.data().unwrap();
    ledger
        .conn
        .execute_batch(
            "CREATE TRIGGER fail_second_sheet BEFORE INSERT ON entries
         WHEN NEW.cents = 6000 BEGIN SELECT RAISE(ABORT, 'test save failure'); END;",
        )
        .unwrap();
    let error = ledger
        .import_excel(&path, selection(), Some(preview.clone()))
        .unwrap_err();
    assert!(error.contains("test save failure"));
    assert_eq!(ledger.data().unwrap(), before);
    assert_eq!(
        ledger.import_excel(&path, selection(), None).unwrap(),
        preview
    );
}

#[test]
fn pickup_batches_count_identical_details_skip_totals_and_track_imported_sheets() {
    let folder = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(folder.path().join("ledger.db")).unwrap();
    let path = folder.path().join("本月_提车费.xlsx");
    let mut workbook = Workbook::new();
    pickup_sheet(&mut workbook, "国产", false);
    pickup_sheet(&mut workbook, "进口", true);
    workbook.add_worksheet();
    workbook.save(&path).unwrap();
    let selection = |sheets: &[&str]| options(sheets, "支出", "提车费");
    let info = ledger.excel_import_info(&path).unwrap();
    assert_eq!(info["sheets"], json!(["国产", "进口"]));
    assert_eq!(info["kind"], "支出");
    assert_eq!(info["default_category"], "提车费");
    let error = ledger
        .import_excel(&path, options(&["国产"], "收入", "运费"), None)
        .unwrap_err();
    assert!(error.contains("只能导入为支出"));
    let preview = ledger
        .import_excel(&path, selection(&["国产", "进口"]), None)
        .unwrap();
    assert_eq!(preview["details"], 3);
    assert_eq!(preview["total"], 15000);
    assert_eq!(preview["entries"].as_array().unwrap().len(), 2);
    assert_eq!(preview["errors"], json!([]));

    let local = ledger
        .import_excel(&path, selection(&["国产"]), None)
        .unwrap();
    ledger
        .import_excel(&path, selection(&["国产"]), Some(local))
        .unwrap();
    let mixed = ledger
        .import_excel(&path, selection(&["国产", "进口"]), None)
        .unwrap();
    assert_eq!(mixed["duplicates"], 2);
    assert_eq!(mixed["total"], 5000);
    ledger
        .import_excel(&path, selection(&["国产", "进口"]), Some(mixed))
        .unwrap();
    let repeated = ledger
        .import_excel(&path, selection(&["国产", "进口"]), None)
        .unwrap();
    assert_eq!(repeated["duplicates"], 3);
    assert_eq!(repeated["entries"], json!([]));

    workbook
        .worksheet_from_name("国产")
        .unwrap()
        .write_number(1, 2, 60)
        .unwrap();
    workbook.save(&path).unwrap();
    let changed = ledger
        .import_excel(&path, selection(&["国产", "进口"]), None)
        .unwrap();
    assert!(changed["errors"][0].as_str().unwrap().contains("「国产」"));
    assert!(ledger
        .import_excel(&path, selection(&["国产", "进口"]), Some(changed))
        .is_err());
    assert_eq!(
        ledger.data().unwrap()["entries"].as_array().unwrap().len(),
        2
    );
}

#[test]
fn pickup_import_reports_missing_headers_and_invalid_amounts() {
    let folder = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(folder.path().join("ledger.db")).unwrap();
    let path = folder.path().join("本月_提车费.xlsx");
    let mut workbook = Workbook::new();
    pickup_sheet(&mut workbook, "国产", false)
        .write_number(1, 2, 0)
        .unwrap();
    freight_sheet(&mut workbook, "其他表", &[("VIN01", "50")]);
    workbook.save(&path).unwrap();
    let info = ledger.excel_import_info(&path).unwrap();
    assert_eq!(info["sheets"], json!(["国产"]));
    let error = ledger
        .import_excel(&path, options(&["其他表"], "支出", "提车费"), None)
        .unwrap_err();
    assert!(error.contains("子表「其他表」需要表头"));
    let preview = ledger
        .import_excel(&path, options(&["国产"], "支出", "提车费"), None)
        .unwrap();
    assert!(preview["errors"][0]
        .as_str()
        .unwrap()
        .contains("「国产」第 2 行"));
    assert!(ledger
        .import_excel(&path, options(&["国产"], "支出", "提车费"), Some(preview))
        .is_err());
}

#[test]
fn existing_ledgers_get_pickup_expense_once_and_keep_it_disabled() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("ledger.db");
    let ledger = Ledger::open(&path).unwrap();
    ledger
        .conn
        .execute_batch(
            "DELETE FROM settings WHERE key='pickup_fee_seeded';
         DELETE FROM categories WHERE kind='支出' AND name='提车费';",
        )
        .unwrap();
    drop(ledger);

    let ledger = Ledger::open(&path).unwrap();
    let pickup = |ledger: &Ledger| {
        ledger.data().unwrap()["categories"]
            .as_array()
            .unwrap()
            .iter()
            .find(|category| category["kind"] == "支出" && category["name"] == "提车费")
            .unwrap()
            .clone()
    };
    assert_eq!(pickup(&ledger)["active"], 1);
    ledger
        .save("categories/delete", json!({"id":pickup(&ledger)["id"]}))
        .unwrap();
    drop(ledger);
    let ledger = Ledger::open(&path).unwrap();
    assert_eq!(pickup(&ledger)["active"], 0);
    let error = ledger
        .excel_import_info(&folder.path().join("本月_提车费.xlsx"))
        .unwrap_err();
    assert!(error.contains("『提车费』不存在或已停用"));
}
