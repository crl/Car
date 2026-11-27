use super::*;
use rust_xlsxwriter::{Workbook, Worksheet};

fn options(sheets: &[&str], kind: &str, category: &str) -> ImportOptions {
    serde_json::from_value(json!({
        "sheets": sheets, "date": "2026-11-24", "party": "测试往来方",
        "kind": kind, "category": category
    }))
    .unwrap()
}

fn payroll_selection(sources: &[(&str, &str)]) -> ImportOptions {
    serde_json::from_value(json!({"sources": sources.iter().map(|(sheet,category)| json!({ "sheet":sheet,"category":category })).collect::<Vec<_>>(),
        "date":"2026-10-31","kind":"支出"})).unwrap()
}

fn payroll_fixture(with_wages: bool, fee: &str) -> Workbook {
    let mut workbook = Workbook::new();
    if with_wages {
        workbook
            .add_worksheet()
            .set_name("工资")
            .unwrap()
            .write_row(0, 1, ["司机工资", "车牌号"])
            .unwrap()
            .write_row(1, 1, ["50", "沪A12345"])
            .unwrap()
            .write_row(2, 1, ["30", "沪A12345"])
            .unwrap();
    }
    let sheet = workbook.add_worksheet().set_name("外包").unwrap();
    sheet
        .write_string(0, 0, "说明")
        .unwrap()
        .write_row(3, 2, ["外包服务费", "车牌号", "司机工资"])
        .unwrap()
        .write_row(4, 2, [fee, "沪A12345", "50"])
        .unwrap()
        .write_row(5, 2, ["1.005", "沪A12345", "30"])
        .unwrap()
        .write_row(6, 2, ["0", "沪B00000", "0"])
        .unwrap()
        .write_formula(
            7,
            2,
            rust_xlsxwriter::Formula::new("=ROUND(SUM(C5:C7),2)").set_result("2.01"),
        )
        .unwrap()
        .write_formula(
            7,
            4,
            rust_xlsxwriter::Formula::new("=SUM(E5:E7)").set_result("80"),
        )
        .unwrap();
    workbook
}

#[test]
fn payroll_adds_only_service_fees_after_a_salary_import() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(dir.path().join("test.db")).unwrap();
    let path = dir.path().join("10月_司机工资.xlsx");
    ledger
        .save("categories", json!({"kind":"支出","name":"司机工资"}))
        .unwrap();
    let mut old = Workbook::new();
    old.add_worksheet()
        .set_name("工资")
        .unwrap()
        .write_row(0, 0, ["车牌号", "司机工资"])
        .unwrap()
        .write_row(1, 0, ["沪A12345", "50"])
        .unwrap()
        .write_row(2, 0, ["沪A12345", "30"])
        .unwrap();
    old.save(&path).unwrap();
    let wages = || options(&["工资"], "支出", "司机工资");
    let first = ledger.import_excel(&path, wages(), None).unwrap();
    ledger.import_excel(&path, wages(), Some(first)).unwrap();
    payroll_fixture(true, "1.005").save(&path).unwrap();
    let info = ledger.excel_import_info(&path).unwrap();
    assert_eq!(
        info["sources"],
        json!([
            {"sheet":"工资","category":"司机工资","enabled":true},
            {"sheet":"外包","category":"外包服务费","enabled":false}
        ])
    );
    let both = || payroll_selection(&[("工资", "司机工资"), ("外包", "外包服务费")]);
    let salary_only = ledger
        .import_excel(&path, payroll_selection(&[("工资", "司机工资")]), None)
        .unwrap();
    assert_eq!(salary_only["duplicates"], 2);
    assert!(ledger
        .import_excel(&path, both(), None)
        .unwrap_err()
        .contains("外包服务费"));
    ledger
        .save("categories", json!({"kind":"支出","name":"外包服务费"}))
        .unwrap();
    let preview = ledger.import_excel(&path, both(), None).unwrap();
    assert_eq!(preview["errors"], json!([]));
    assert_eq!(preview["duplicates"], 2);
    assert_eq!(preview["total"], 201);
    assert_eq!(preview["entries"][0]["note"], "由2笔合并");
    assert_eq!(preview["entries"][0]["category"], "外包服务费");
    assert!(ledger
        .import_excel(&path, payroll_selection(&[("外包", "司机工资")]), None)
        .is_err());
    ledger.import_excel(&path, both(), Some(preview)).unwrap();
    let again = ledger.import_excel(&path, both(), None).unwrap();
    assert_eq!(again["duplicates"], 4);
    assert_eq!(again["entries"], json!([]));
    payroll_fixture(true, "1.006").save(&path).unwrap();
    assert!(!ledger.import_excel(&path, both(), None).unwrap()["errors"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn payroll_mixed_sheet_uses_two_projects_and_commits_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(dir.path().join("test.db")).unwrap();
    let path = dir.path().join("10月_司机工资.xlsx");
    for name in ["司机工资", "外包服务费"] {
        ledger
            .save("categories", json!({"kind":"支出","name":name}))
            .unwrap();
    }
    payroll_fixture(false, "1.005").save(&path).unwrap();
    let both = || payroll_selection(&[("外包", "司机工资"), ("外包", "外包服务费")]);
    assert_eq!(
        ledger.excel_import_info(&path).unwrap()["sources"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let first = ledger.import_excel(&path, both(), None).unwrap();
    assert_eq!(first["total"], 8201);
    payroll_fixture(false, "1.006").save(&path).unwrap();
    assert!(ledger.import_excel(&path, both(), Some(first)).is_err());
    assert_eq!(ledger.data().unwrap()["entries"], json!([]));
    let preview = ledger.import_excel(&path, both(), None).unwrap();
    ledger.conn.execute_batch("CREATE TRIGGER fail_service BEFORE INSERT ON excel_service_fee_sources BEGIN SELECT RAISE(ABORT,'test'); END;").unwrap();
    assert!(ledger
        .import_excel(&path, both(), Some(preview.clone()))
        .is_err());
    assert_eq!(ledger.import_excel(&path, both(), None).unwrap(), preview);
    ledger
        .conn
        .execute_batch("DROP TRIGGER fail_service")
        .unwrap();
    assert_eq!(
        ledger.import_excel(&path, both(), Some(preview)).unwrap()["count"],
        2
    );
    assert_eq!(
        ledger
            .conn
            .query_row(
                "SELECT cents FROM entries WHERE category='外包服务费'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        201
    );
    assert_eq!(
        ledger
            .conn
            .query_row(
                "SELECT cents FROM entries WHERE category='司机工资'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        8000
    );
    // A plain salary sheet is added later, with the same wages already paid from the mixed sheet.
    payroll_fixture(true, "1.006").save(&path).unwrap();
    let later = ledger
        .import_excel(
            &path,
            payroll_selection(&[("工资", "司机工资"), ("外包", "外包服务费")]),
            None,
        )
        .unwrap();
    assert_eq!(later["entries"], json!([]));
    assert_eq!(later["duplicates"], 4);
}

#[test]
fn payroll_fee_errors_identify_bad_rows_and_allow_salary_only() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(dir.path().join("test.db")).unwrap();
    let path = dir.path().join("当月_司机工资.xlsx");
    for name in ["司机工资", "外包服务费"] {
        ledger
            .save("categories", json!({"kind":"支出","name":name}))
            .unwrap();
    }
    let mut book = payroll_fixture(false, "1.005");
    let sheet = book.add_worksheet().set_name("服务费").unwrap();
    sheet
        .set_formula_result_default("")
        .write_row(2, 2, ["车牌号", "外包服务费"])
        .unwrap()
        .write_number(3, 3, 100)
        .unwrap()
        .write_row(4, 2, ["沪A", "-25"])
        .unwrap()
        .write_string(5, 2, "沪B")
        .unwrap()
        .write_formula(5, 3, "=1+2")
        .unwrap();
    book.save(&path).unwrap();
    let bad = || payroll_selection(&[("外包", "司机工资"), ("服务费", "外包服务费")]);
    let preview = ledger.import_excel(&path, bad(), None).unwrap();
    let errors = preview["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 3);
    for (i, error) in errors.iter().enumerate() {
        assert!(error
            .as_str()
            .unwrap()
            .contains(&format!("「服务费」第 {} 行", 4 + i)));
    }
    assert!(ledger.import_excel(&path, bad(), Some(preview)).is_err());
    assert_eq!(ledger.data().unwrap()["entries"], json!([]));
    assert_eq!(
        ledger
            .import_excel(&path, payroll_selection(&[("外包", "司机工资")]), None)
            .unwrap()["errors"],
        json!([])
    );
}

fn salary_workbook(wage: &str) -> Workbook {
    let mut workbook = Workbook::new();
    let sheet = workbook.add_worksheet().set_name("工资明细").unwrap();
    sheet
        .write_string(1, 1, "本月工资")
        .unwrap()
        .write_string(4, 1, "司机工资")
        .unwrap()
        .write_string(4, 4, "车牌号")
        .unwrap()
        .write_formula(
            5,
            1,
            rust_xlsxwriter::Formula::new("=70+30").set_result(wage),
        )
        .unwrap()
        .write_string(5, 4, " 沪a12345 ")
        .unwrap()
        .write_number(6, 1, 80.25)
        .unwrap()
        .write_string(6, 4, "沪A12345")
        .unwrap()
        .write_number(7, 1, 0)
        .unwrap()
        .write_string(7, 4, "沪B00000")
        .unwrap()
        .write_string(8, 0, "其他员工")
        .unwrap()
        .write_formula(
            9,
            1,
            rust_xlsxwriter::Formula::new("=SUM(B6:B9)").set_result("180.25"),
        )
        .unwrap();
    workbook
        .add_worksheet()
        .set_name("另一张")
        .unwrap()
        .write_row(0, 0, ["车牌号", "司机工资"])
        .unwrap()
        .write_row(1, 0, ["沪A12345", "20"])
        .unwrap();
    workbook
        .add_worksheet()
        .set_name("不匹配")
        .unwrap()
        .write_row(0, 0, ["车牌号", "实发工资"])
        .unwrap()
        .write_string(1, 1, "司机工资")
        .unwrap();
    workbook
}

#[test]
fn salary_matches_sheets_groups_per_plate_and_tracks_each_imported_sheet() {
    let folder = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(folder.path().join("test.db")).unwrap();
    let path = folder.path().join("本月_司机工资.xlsx");
    salary_workbook("100").save(&path).unwrap();
    let unavailable = || {
        assert!(ledger
            .excel_import_info(&path)
            .unwrap_err()
            .contains("『司机工资』不存在或已停用"))
    };
    unavailable();
    ledger
        .save("categories", json!({"kind":"支出","name":"司机工资"}))
        .unwrap();
    ledger
        .conn
        .execute("UPDATE categories SET active=0 WHERE name='司机工资'", [])
        .unwrap();
    unavailable();
    ledger
        .save("categories", json!({"kind":"支出","name":"司机工资"}))
        .unwrap();
    let info = ledger.excel_import_info(&path).unwrap();
    assert_eq!(info["sheets"], json!(["工资明细", "另一张"]));
    assert_eq!(info["default_category"], "司机工资");
    assert_eq!(info["kind"], "支出");
    assert_eq!(info["requires_party"], false);
    let select = || ImportOptions {
        party: String::new(),
        ..options(&["工资明细", "另一张"], "支出", "司机工资")
    };
    assert!(ledger
        .import_excel(&path, options(&["不匹配"], "支出", "司机工资"), None)
        .is_err());
    let preview = ledger.import_excel(&path, select(), None).unwrap();
    assert_eq!(preview["details"], 3);
    assert_eq!(preview["entries"][0]["sources"], 2);
    assert_eq!(preview["entries"][0]["cents"], 18025);
    assert_eq!(preview["total"], 20025);
    assert_eq!(preview["new_plates"], json!(["沪A12345"]));
    assert_eq!(preview["new_addresses"], json!([]));
    salary_workbook("101").save(&path).unwrap();
    assert!(ledger
        .import_excel(&path, select(), Some(preview.clone()))
        .is_err());
    assert_eq!(ledger.data().unwrap()["entries"], json!([]));
    salary_workbook("100").save(&path).unwrap();
    assert_eq!(
        ledger.import_excel(&path, select(), Some(preview)).unwrap()["count"],
        2
    );
    let repeated = ledger.import_excel(&path, select(), None).unwrap();
    assert!(ledger.data().unwrap()["entries"]
        .as_array()
        .unwrap()
        .iter()
        .all(|entry| entry["party"] == ""));
    assert_eq!(repeated["entries"], json!([]));
    assert_eq!(repeated["duplicates"], 3);
    salary_workbook("102").save(&path).unwrap();
    let conflict = ledger.import_excel(&path, select(), None).unwrap();
    assert!(conflict["errors"][0].as_str().unwrap().contains("工资明细"));
    assert!(ledger
        .import_excel(&path, select(), Some(conflict))
        .is_err());
}

#[test]
fn invalid_salaries_report_excel_row_numbers_and_block_the_batch() {
    let folder = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(folder.path().join("test.db")).unwrap();
    ledger
        .save("categories", json!({"kind":"支出","name":"司机工资"}))
        .unwrap();
    let path = folder.path().join("测试_司机工资.xlsx");
    let mut workbook = salary_workbook("100");
    let sheet = workbook.add_worksheet().set_name("问题").unwrap();
    sheet
        .set_formula_result_default("")
        .write_row(4, 3, ["车牌号", "司机工资"])
        .unwrap()
        .write_string(5, 3, "沪A1")
        .unwrap()
        .write_number(6, 4, 120)
        .unwrap()
        .write_row(7, 3, ["沪A2", "-1"])
        .unwrap()
        .write_row(8, 3, ["沪A3", "文字"])
        .unwrap()
        .write_row(9, 3, ["沪A4", "0.001"])
        .unwrap()
        .write_row(10, 3, ["沪A5", "0.00"])
        .unwrap()
        .write_formula(11, 4, "=1+2")
        .unwrap();
    workbook.save(&path).unwrap();
    let select = || options(&["问题", "工资明细"], "支出", "司机工资");
    let preview = ledger.import_excel(&path, select(), None).unwrap();
    let errors = preview["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 6, "{errors:?}");
    for (error, row) in errors.iter().zip([6, 7, 8, 9, 10, 12]) {
        assert!(error
            .as_str()
            .unwrap()
            .contains(&format!("「问题」第 {row} 行")));
    }
    assert!(errors[5].as_str().unwrap().contains("计算结果"));
    assert!(ledger.import_excel(&path, select(), Some(preview)).is_err());
    assert_eq!(ledger.data().unwrap()["entries"], json!([]));
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

fn transfer_sheet<'a>(
    workbook: &'a mut Workbook,
    name: &str,
    rows: &[(&str, &str)],
) -> &'a mut Worksheet {
    let sheet = workbook.add_worksheet().set_name(name).unwrap();
    sheet
        .write_row(
            0,
            0,
            [
                "派车单编号",
                "车牌号",
                "客户名称",
                "目的地",
                "公司付倒板费",
                "装车地",
                "预付油款",
            ],
        )
        .unwrap();
    for (index, &(dispatch, amount)) in rows.iter().enumerate() {
        sheet
            .write_row(
                index as u32 + 1,
                0,
                [
                    dispatch,
                    "沪A12345",
                    "表内客户",
                    "泉州",
                    amount,
                    "金华",
                    "800",
                ],
            )
            .unwrap();
    }
    sheet
}

#[test]
fn transfer_fees_skip_footer_group_and_deduplicate_dispatches() {
    let folder = tempfile::tempdir().unwrap();
    let db = folder.path().join("ledger.db");
    let ledger = Ledger::open(&db).unwrap();
    let path = folder.path().join("10月份_倒板费.xlsx");
    let mut workbook = Workbook::new();
    let sheet = transfer_sheet(&mut workbook, "甲", &[("001", "480"), ("002", "120.50")]);
    // Leave a blank row before the hardcoded, multi-column footer.
    sheet
        .write_number(4, 4, 51680)
        .unwrap()
        .write_number(4, 6, 927316.45)
        .unwrap();
    transfer_sheet(&mut workbook, "乙", &[("001", "480"), ("003", "720")]);
    freight_sheet(&mut workbook, "其他", &[]);
    workbook.save(&path).unwrap();
    let info = ledger.excel_import_info(&path).unwrap();
    assert_eq!(info["sheets"], json!(["甲", "乙"]));
    assert_eq!(info["kind"], "支出");
    assert_eq!(info["default_category"], "倒板费");
    assert_eq!(info["requires_party"], true);
    let select = || options(&["甲", "乙"], "支出", "倒板费");
    let missing_party = ImportOptions {
        party: "  ".into(),
        ..select()
    };
    assert_eq!(
        ledger.import_excel(&path, missing_party, None).unwrap_err(),
        "请填写往来方"
    );
    let preview = ledger.import_excel(&path, select(), None).unwrap();
    assert_eq!(preview["details"], 4);
    assert_eq!(preview["duplicates"], 1);
    assert_eq!(preview["total"], 132050);
    assert_eq!(preview["entries"][0]["sources"], 2);
    assert_eq!(preview["entries"][0]["note"], "由2笔合并");
    assert_eq!(preview["entries"][1]["note"], "");
    let saved = ledger.import_excel(&path, select(), Some(preview)).unwrap();
    assert_eq!(saved["count"], 2);
    let entries = ledger.data().unwrap()["entries"].clone();
    for entry in entries.as_array().unwrap() {
        assert_eq!(
            entry["note"],
            if entry["cents"] == 60050 {
                "由2笔合并"
            } else {
                ""
            }
        );
    }
    assert!(entries
        .as_array()
        .unwrap()
        .iter()
        .all(|entry| entry["party"] == "测试往来方" && entry["date"] == "2026-11-24"));
    drop(ledger);
    let ledger = Ledger::open(db).unwrap();
    let repeated = ledger.import_excel(&path, select(), None).unwrap();
    assert_eq!(repeated["duplicates"], 4);
    assert_eq!(repeated["entries"], json!([]));
    assert_eq!(repeated["total"], 0);

    let other = folder.path().join("补录_倒板费.xlsx");
    let mut workbook = Workbook::new();
    transfer_sheet(&mut workbook, "乙", &[("004", "50"), ("001", "481")]);
    workbook.save(&other).unwrap();
    let select = || options(&["乙"], "支出", "倒板费");
    let conflict = ledger.import_excel(&other, select(), None).unwrap();
    assert!(conflict["errors"][0]
        .as_str()
        .unwrap()
        .contains("「乙」第 3 行：派车单编号「001」重复"));
    assert!(ledger
        .import_excel(&other, select(), Some(conflict))
        .is_err());
    assert_eq!(ledger.data().unwrap()["entries"], entries);
}

#[test]
fn transfer_fees_reject_invalid_rows_headers_conflicts_and_stale_previews() {
    let folder = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(folder.path().join("ledger.db")).unwrap();
    let path = folder.path().join("本月_倒板费.xlsx");
    let select = || options(&["明细"], "支出", "倒板费");
    let write = |rows: &[(&str, &str)]| {
        let mut workbook = Workbook::new();
        transfer_sheet(&mut workbook, "明细", rows);
        workbook.save(&path).unwrap();
    };
    write(&[("001", "50")]);
    let preview = ledger.import_excel(&path, select(), None).unwrap();
    // Same amount and route, but the dispatch that would be recorded has changed.
    write(&[("002", "50")]);
    assert!(ledger.import_excel(&path, select(), Some(preview)).is_err());

    let mut workbook = Workbook::new();
    let sheet = transfer_sheet(
        &mut workbook,
        "明细",
        &[("001", "50"), ("001", "60"), ("002", "-1"), ("", "50")],
    );
    sheet.write_number(5, 4, 50).unwrap(); // An interior amount-only row is invalid.
    sheet.write_string(6, 2, "仅有客户名称").unwrap();
    workbook.save(&path).unwrap();
    let preview = ledger.import_excel(&path, select(), None).unwrap();
    let errors = preview["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 5);
    for (index, problem) in errors.iter().enumerate() {
        assert!(problem
            .as_str()
            .unwrap()
            .contains(&format!("「明细」第 {} 行", index + 3)));
    }
    assert!(ledger.import_excel(&path, select(), Some(preview)).is_err());
    assert_eq!(ledger.data().unwrap()["entries"], json!([]));

    let mut workbook = Workbook::new();
    freight_sheet(&mut workbook, "明细", &[]);
    workbook.save(&path).unwrap();
    assert!(ledger
        .excel_import_info(&path)
        .unwrap_err()
        .contains("公司付倒板费"));
    assert!(ledger
        .import_excel(&path, select(), None)
        .unwrap_err()
        .contains("派车单编号"));
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
fn pickup_filename_marker_can_appear_anywhere_in_the_file_name() {
    let folder = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(folder.path().join("ledger.db")).unwrap();
    let mut workbook = Workbook::new();
    pickup_sheet(&mut workbook, "国产", false);

    for name in ["本月_提车费..xlsx", "本月_提车费_补录.xlsx", "_提车费.xlsx"] {
        let path = folder.path().join(name);
        workbook.save(&path).unwrap();
        let info = ledger.excel_import_info(&path).unwrap();
        assert_eq!(info["kind"], "支出", "{name}");
        assert_eq!(info["default_category"], "提车费", "{name}");
        let preview = ledger
            .import_excel(&path, options(&["国产"], "支出", "提车费"), None)
            .unwrap();
        assert_eq!(preview["errors"], json!([]), "{name}");
        assert_eq!(preview["total"], 10000, "{name}");
    }
}

#[test]
fn unregistered_and_ambiguous_names_are_rejected_before_opening_excel() {
    let folder = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(folder.path().join("ledger.db")).unwrap();
    for name in ["本月_提车费_运费.xlsx", "本月_油费.xlsx"] {
        let path = folder.path().join(name);
        let picker_error = ledger.excel_import_info(&path).unwrap_err();
        let preview_error = ledger
            .import_excel(&path, options(&["明细"], "收入", "运费"), None)
            .unwrap_err();
        assert_eq!(preview_error, picker_error);
        if name.contains("油费") {
            assert!(
                picker_error.contains("暂不支持") && picker_error.contains("油费"),
                "{picker_error}"
            );
        } else {
            assert!(
                picker_error.contains("提车费")
                    && picker_error.contains("运费")
                    && picker_error.contains("修改文件名"),
                "{picker_error}"
            );
        }
    }
}

#[test]
fn a_registered_handler_can_inspect_preview_and_commit_its_own_format() {
    const HEADERS: &[&str] = &["燃油金额", "车号"];

    struct FuelImporter;
    struct FuelRun;

    impl ExcelImporter for FuelImporter {
        fn kind(&self) -> &'static str {
            "支出"
        }

        fn headers(&self) -> &'static [&'static str] {
            HEADERS
        }

        fn start(&self, _file_stem: &str) -> Box<dyn ImportRun> {
            Box::new(FuelRun)
        }
    }

    impl ImportRun for FuelRun {
        fn read_sheet(
            &mut self,
            _tx: &Transaction<'_>,
            workbook: &mut super::Workbook,
            sheet: &str,
            _errors: &mut Vec<String>,
        ) -> Result<PreparedSheet> {
            let (range, columns) = read_table(workbook, sheet, HEADERS)?;
            let first_row = range.start().unwrap().0 as usize;
            let rows = range
                .rows()
                .enumerate()
                .skip(1)
                .map(|(index, row)| {
                    Ok(Detail {
                        merged_rows: Vec::new(),
                        row: first_row + index + 1,
                        excluded: false,
                        brand: String::new(),
                        party: None,
                        start: String::new(),
                        end: String::new(),
                        plate: field(row, columns[1]).to_uppercase(),
                        cents: amount(row.get(columns[0]))?,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(PreparedSheet::new(fingerprint(&rows)?, rows.len(), rows))
        }

        fn record_sources(
            &self,
            tx: &Transaction<'_>,
            sheet: &SheetImport,
            group: &Group,
            entry_id: i64,
        ) -> Result<()> {
            tx.execute(
                "INSERT INTO test_fuel_sources VALUES(?,?,?)",
                params![entry_id, sheet.name, group.source_rows[0] as i64],
            )
            .map_err(err)?;
            Ok(())
        }
    }

    let folder = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(folder.path().join("ledger.db")).unwrap();
    ledger.conn.execute_batch(
        "CREATE TABLE test_fuel_sources(entry_id INTEGER REFERENCES entries(id), sheet TEXT, row_number INTEGER);",
    ).unwrap();
    let mut registry = ImportRegistry::default();
    registry.register("油费", FuelImporter).unwrap();
    assert!(registry.register("油费", FuelImporter).is_err());

    let path = folder.path().join("本月_油费.xlsx");
    let mut workbook = Workbook::new();
    workbook
        .add_worksheet()
        .set_name("燃油明细")
        .unwrap()
        .write_row(0, 0, HEADERS.iter().copied())
        .unwrap()
        .write_row(1, 0, ["19.50", "沪A12345"])
        .unwrap();
    freight_sheet(&mut workbook, "不同表头", &[("VIN01", "60")]);
    workbook.save(&path).unwrap();

    let info = registry.inspect(&ledger, &path).unwrap();
    assert_eq!(info["sheets"], json!(["燃油明细"]));
    assert_eq!(info["kind"], "支出");
    assert_eq!(info["default_category"], "油费");
    let selection = || options(&["燃油明细"], "支出", "油费");
    let preview = registry.import(&ledger, &path, selection(), None).unwrap();
    assert_eq!(preview["total"], 1950);
    assert_eq!(preview["new_addresses"], json!([]));
    let result = registry
        .import(&ledger, &path, selection(), Some(preview))
        .unwrap();
    assert_eq!(result["count"], 1);
    let source = ledger
        .conn
        .query_row(
            "SELECT e.category, e.cents, s.sheet, s.row_number
         FROM entries e JOIN test_fuel_sources s ON e.id=s.entry_id",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(source, ("油费".into(), 1950, "燃油明细".into(), 2));
    assert_eq!(
        ledger.data().unwrap()["entries"][0]["start_address_id"],
        Value::Null
    );
}

#[test]
fn existing_freight_and_pickup_source_records_still_prevent_duplicate_entries() {
    let folder = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(folder.path().join("ledger.db")).unwrap();
    // These are source records as persisted by v0.1.8, independently of the import handlers.
    ledger
        .conn
        .execute_batch(
            "INSERT INTO cars(id,name,plate) VALUES(1,'','沪A12345');
         INSERT INTO entries(id,car_id,date,kind,category,cents) VALUES
            (1,1,'2026-11-24','收入','运费',5000), (2,1,'2026-11-24','支出','提车费',10000);",
        )
        .unwrap();
    ledger.conn.execute(
        "INSERT INTO excel_import_sources(vin,order_number,fingerprint,entry_id) VALUES('VIN01','单号01',?,1)",
        [r#"["沪A12345","上海","昆山",5000]"#],
    ).unwrap();
    ledger.conn.execute(
        "INSERT INTO excel_pickup_sources(file_stem,sheet,fingerprint,entry_id) VALUES('本月_提车费','明细',?,2)",
        ["1a0f2acd5e77767f94bd5621a109f0500c61ad81c9f35b5a2ddf2180c8df151e"],
    ).unwrap();

    let mut freight = Workbook::new();
    freight_sheet(&mut freight, "明细", &[("VIN01", "50")]);
    let mut pickup = Workbook::new();
    pickup_sheet(&mut pickup, "明细", false);
    for (mut workbook, kind, name, duplicates) in
        [(freight, "收入", "运费", 1), (pickup, "支出", "提车费", 2)]
    {
        let path = folder.path().join(format!("本月_{name}.xlsx"));
        workbook.save(&path).unwrap();
        let preview = ledger
            .import_excel(&path, options(&["明细"], kind, name), None)
            .unwrap();
        assert_eq!(preview["errors"], json!([]), "{name}");
        assert_eq!(preview["duplicates"], duplicates, "{name}");
        assert_eq!(preview["entries"], json!([]), "{name}");
    }
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

#[test]
fn modern_freight_groups_by_brand_and_customer_and_skips_invalid_rows() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(dir.path().join("test.db")).unwrap();
    let path = dir.path().join("测试_运费.xlsx");
    let mut workbook = Workbook::new();
    let sheet = workbook.add_worksheet().set_name("明细").unwrap();
    sheet.write_row(0, 0, ["品牌","客户名称","车架号","运费收入","目的地城市","车牌号","装车地城市"]).unwrap();
    for (index, row) in [
        ["甲品牌","客户甲","VIN1","100","上海","沪A12345","南京"],
        ["甲品牌","客户甲","VIN1","100","上海","沪A12345","南京"],
        ["乙品牌","客户甲","VIN1","50","上海","沪A12345","南京"],
        ["甲品牌","客户乙","VIN1","30","上海","沪A12345","南京"],
        ["","客户甲","VIN2","20","上海","沪A12345","南京"],
        ["甲品牌","客户甲","VIN3","50","上海","","南京"],
        ["甲品牌","客户甲","VIN4","不详","上海","沪A12345","南京"],
        ["甲品牌","","VIN5","50","上海","沪A12345","南京"],
    ].into_iter().enumerate() {
        sheet.write_row(index as u32 + 1, 0, row).unwrap();
    }
    workbook.save(&path).unwrap();
    let request = || serde_json::from_value(json!({
        "sheets":["明细"], "date":"2026-10-31", "kind":"收入","category":"运费"
    })).unwrap();
    assert_eq!(ledger.excel_import_info(&path).unwrap()["requires_party"], false);
    let preview = ledger.import_excel(&path, request(), None).unwrap();
    assert_eq!(preview["errors"], json!([]));
    assert_eq!(preview["warnings"].as_array().unwrap().len(), 3);
    assert_eq!(preview["total"], 30000);
    assert_eq!(preview["entries"].as_array().unwrap().len(), 4);
    assert_eq!(preview["entries"][0]["sources"], 2);
    assert_eq!(preview["entries"][0]["brand"], "甲品牌");
    ledger.import_excel(&path, request(), Some(preview)).unwrap();
    let data=ledger.data().unwrap();
    let entries=data["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 4);
    assert!(entries.iter().all(|e| e["date"] == "2026-10-31"));
    assert!(entries.iter().any(|e| e["party"] == "客户乙" && e["brand"] == "甲品牌"));
    let duplicate=ledger.import_excel(&path, request(), None).unwrap();
    assert_eq!(duplicate["entries"], json!([]));
    assert!(ledger.import_excel(&path, request(), Some(duplicate)).is_err());
}

#[test]
#[ignore = "set CAR_FREIGHT_SAMPLE to a local modern workbook"]
fn modern_freight_workbook_smoke() {
    let path = std::path::PathBuf::from(std::env::var_os("CAR_FREIGHT_SAMPLE").unwrap());
    let dir = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(dir.path().join("sample.db")).unwrap();
    let info = ledger.excel_import_info(&path).unwrap();
    assert_eq!(info["requires_party"], false);
    let request = || serde_json::from_value(json!({
        "sheets":info["sheets"], "date":"2026-10-31", "kind":"收入","category":"运费"
    })).unwrap();
    let preview = ledger.import_excel(&path, request(), None).unwrap();
    assert_eq!(preview["errors"], json!([]));
    let rows=preview["entries"].as_array().unwrap();
    let total: i64=rows.iter().map(|row| row["cents"].as_i64().unwrap()).sum();
    assert_eq!(preview["total"], total);
    assert_eq!(preview["source_total"], total);
    assert!(rows.iter().all(|row| !row["party"].as_str().unwrap().is_empty()));
    println!("details={}, warnings={}, groups={}, cents={}",preview["details"],preview["warnings"].as_array().unwrap().len(),rows.len(),total);
    ledger.import_excel(&path, request(), Some(preview)).unwrap();
    let data=ledger.data().unwrap();
    assert!(data["entries"].as_array().unwrap().iter().all(|entry| entry["date"]=="2026-10-31"));
}

#[test]
fn address_aliases_are_unique_and_excel_reuses_the_formal_address() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(dir.path().join("test.db")).unwrap();
    ledger.save("addresses", json!({"name":"南京","aliases":[" NJ ", "nj", "", "金陵"]})).unwrap();
    ledger.save("addresses", json!({"id":1,"name":"南京","detail":"保留别名"})).unwrap();
    assert_eq!(ledger.data().unwrap()["addresses"][0]["aliases"], json!(["NJ","金陵"]));
    assert!(ledger.save("addresses", json!({"name":"Nj"})).is_err());
    assert!(ledger.save("addresses", json!({"name":"苏州","aliases":["南京"]})).is_err());
    assert!(ledger.save("addresses", json!({"name":"苏州","aliases":["金陵"]})).is_err());
    let path = dir.path().join("别名_运费.xlsx");
    let mut book = Workbook::new();
    let sheet = book.add_worksheet().set_name("明细").unwrap();
    sheet.write_row(0, 0, ["装车地城市","目的地城市","运费收入","车牌号","客户名称","品牌"]).unwrap();
    sheet.write_row(1, 0, ["南京","上海","50","沪A12345","客户","品牌"]).unwrap();
    sheet.write_row(2, 0, [" nj ","上海","30","沪A12345","客户","品牌"]).unwrap();
    book.save(&path).unwrap();
    let request = || options(&["明细"], "收入", "运费");
    let preview = ledger.import_excel(&path, request(), None).unwrap();
    assert_eq!(preview["entries"].as_array().unwrap().len(), 1);
    assert_eq!(preview["entries"][0]["start"], "南京");
    assert_eq!(preview["total"], 8000);
    assert_eq!(preview["new_addresses"], json!(["上海"]));
    ledger.import_excel(&path, request(), Some(preview)).unwrap();
    let data = ledger.data().unwrap();
    assert_eq!(data["entries"][0]["start_address"], "南京");
    assert_eq!(data["addresses"].as_array().unwrap().len(), 2);
    ledger.save("addresses", json!({"id":1,"name":"南京","aliases":[]})).unwrap();
    assert_eq!(ledger.data().unwrap()["entries"][0]["start_address"], "南京");
}

#[test]
fn duplicate_city_addresses_merge_atomically_without_rewriting_route_labels() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(dir.path().join("test.db")).unwrap();
    for (name, detail, aliases) in [
        ("莆田", "站点", vec!["旧站"]),
        ("莆田市", "", vec!["莆田城区"]),
        ("金华", "", vec![]),
        ("金华市", "", vec![]),
        ("苏州", "甲站", vec![]),
        ("苏州市", "乙站", vec![]),
    ] {
        ledger.save("addresses", json!({"name":name,"detail":detail,"aliases":aliases})).unwrap();
    }
    ledger.save("cars", json!({"plate":"闽A12345"})).unwrap();
    ledger.save("entries", json!({"car_id":1,"date":"2026-11-28","kind":"收入","category":"运费","amount":"10",
        "start_address_id":1,"end_address_id":3,"start_address_detail":"1号门"})).unwrap();
    let snapshot = ledger.data().unwrap()["entries"][0].clone();
    let preview = ledger.save("addresses/merge-preview", json!({})).unwrap();
    assert_eq!(preview["pairs"].as_array().unwrap().len(),3);
    let cities = || json!({"pairs":[{"source_id":1,"target_id":2},{"source_id":3,"target_id":4}]});
    assert!(ledger.save("addresses/merge",json!({"pairs":[{"source_id":1,"target_id":2},{"source_id":5,"target_id":6}]})).is_err());
    assert_eq!(ledger.data().unwrap()["entries"][0]["start_address_id"],1);
    ledger.conn.execute("UPDATE addresses SET aliases=? WHERE id=6",["[\"旧站\"]"]).unwrap();
    assert!(ledger.save("addresses/merge",cities()).is_err());
    ledger.conn.execute("UPDATE addresses SET aliases='[]' WHERE id=6",[]).unwrap();
    assert_eq!(ledger.save("addresses/merge",cities()).unwrap()["merged"],2);
    assert!(ledger.save("addresses/merge",cities()).is_err());
    let data = ledger.data().unwrap();
    let addresses=data["addresses"].as_array().unwrap();
    let address=|id| addresses.iter().find(|row| row["id"]==id).unwrap();
    assert_eq!(address(1)["active"],0);
    assert_eq!(address(2)["aliases"],json!(["莆田城区","莆田","旧站"]));
    assert_eq!(address(2)["detail"],"站点");
    assert_eq!(data["entries"][0]["start_address_id"],2);
    assert_eq!(data["entries"][0]["end_address_id"],4);
    for field in ["start_address","end_address","start_address_detail"] {
        assert_eq!(data["entries"][0][field],snapshot[field]);
    }
    let path=dir.path().join("旧称_运费.xlsx");
    let mut book=Workbook::new();
    book.add_worksheet().set_name("明细").unwrap()
        .write_row(0,0,["装车地城市","目的地城市","运费收入","车牌号","客户名称","品牌"]).unwrap()
        .write_row(1,0,["莆田","金华","2","闽A12345","客户",""]).unwrap();
    book.save(&path).unwrap();
    let preview=ledger.import_excel(&path,options(&["明细"],"收入","运费"),None).unwrap();
    assert_eq!(preview["new_addresses"],json!([]));
    assert_eq!(preview["entries"][0]["start"],"莆田市");
    assert_eq!(preview["entries"][0]["end"],"金华市");
}
