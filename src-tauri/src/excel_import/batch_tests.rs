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
    let select = || options(&["甲", "乙"], "支出", "倒板费");
    let preview = ledger.import_excel(&path, select(), None).unwrap();
    assert_eq!(preview["details"], 4);
    assert_eq!(preview["duplicates"], 1);
    assert_eq!(preview["total"], 132050);
    assert_eq!(preview["entries"][0]["sources"], 2);
    let saved = ledger.import_excel(&path, select(), Some(preview)).unwrap();
    assert_eq!(saved["count"], 2);
    let entries = ledger.data().unwrap()["entries"].clone();
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
                        row: first_row + index + 1,
                        excluded: false,
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
