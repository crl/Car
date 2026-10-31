use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
type Result<T> = std::result::Result<T, String>;
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
pub struct Ledger {
    conn: Connection,
    path: PathBuf,
}
fn rows(c: &Connection, sql: &str) -> Result<Vec<Value>> {
    let mut s = c.prepare(sql).map_err(err)?;
    let names: Vec<String> = s.column_names().iter().map(|s| s.to_string()).collect();
    let r = s
        .query_map([], |r| {
            let mut o = serde_json::Map::new();
            for (i, n) in names.iter().enumerate() {
                use rusqlite::types::ValueRef;
                let v = match r.get_ref(i)? {
                    ValueRef::Null => Value::Null,
                    ValueRef::Integer(v) => json!(v),
                    ValueRef::Real(v) => json!(v),
                    ValueRef::Text(v) => json!(String::from_utf8_lossy(v)),
                    ValueRef::Blob(_) => Value::Null,
                };
                o.insert(n.clone(), v);
            }
            Ok(Value::Object(o))
        })
        .map_err(err)?;
    r.collect::<std::result::Result<Vec<_>, _>>().map_err(err)
}
fn text(d: &Value, k: &str) -> String {
    match &d[k] {
        Value::String(s) => s.trim().into(),
        Value::Null => String::new(),
        v => v.to_string(),
    }
}
fn id(d: &Value, k: &str) -> Result<Option<i64>> {
    if d[k].is_null() || d[k] == 0 || d[k] == "" {
        return Ok(None);
    }
    d[k].as_i64()
        .or_else(|| d[k].as_str().and_then(|s| s.parse().ok()))
        .map(Some)
        .ok_or_else(|| "无效编号".into())
}
fn cents(v: &Value) -> Result<i64> {
    let s = match v {
        Value::String(s) => s.trim().to_owned(),
        Value::Number(n) => n.to_string(),
        _ => return Err("金额须大于零，最多两位小数".into()),
    };
    let fail = || "金额须大于零，最多两位小数".to_string();
    let s = s.strip_prefix('+').unwrap_or(&s);
    let parts: Vec<_> = s.split(['e', 'E']).collect();
    if parts.len() > 2 {
        return Err(fail());
    }
    let exp: i32 = if parts.len() == 2 {
        parts[1].parse().map_err(|_| fail())?
    } else {
        0
    };
    let m: Vec<_> = parts[0].split('.').collect();
    if m.len() > 2 {
        return Err(fail());
    }
    let digits = m.join("");
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(fail());
    }
    let digits = digits.trim_start_matches('0');
    if digits.is_empty() {
        return Err(fail());
    }
    let scale = 2i64 + exp as i64 - m.get(1).map_or(0, |s| s.len()) as i64;
    let mut n = digits.to_owned();
    if scale < 0 {
        let cut = (-scale) as usize;
        if cut >= n.len() || !n[n.len() - cut..].bytes().all(|b| b == b'0') {
            return Err(fail());
        }
        n.truncate(n.len() - cut);
    } else {
        if scale > 12 {
            return Err(fail());
        }
        n.push_str(&"0".repeat(scale as usize));
    }
    let n: i64 = n.parse().map_err(|_| fail())?;
    if n <= 0 || n > 99999999900 {
        Err(fail())
    } else {
        Ok(n)
    }
}
impl Ledger {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if path.exists() && std::fs::metadata(&path).map_err(err)?.len() > 0 {
            drop(validated_inner(&path, false)?);
        }
        let conn = Connection::open(&path).map_err(err)?;
        conn.execute_batch("PRAGMA foreign_keys=ON;").map_err(err)?;
        let existing = rows(
            &conn,
            "SELECT name FROM sqlite_master WHERE type='table' AND name='entries'",
        )?;
        if !existing.is_empty()
            && [
                ("entries", "start_address_id"),
                ("entries", "end_address_id"),
                ("entries", "start_address"),
                ("entries", "end_address"),
                ("categories", "requires_route"),
                ("addresses", "region_code"),
            ]
            .iter()
            .any(|(t, n)| {
                rows(&conn, &format!("PRAGMA table_info({t})"))
                    .map(|v| v.iter().all(|r| r["name"] != *n))
                    .unwrap_or(true)
            })
        {
            let dir = path.parent().unwrap_or(Path::new(".")).join("backups");
            std::fs::create_dir_all(&dir).map_err(err)?;
            conn.backup(
                rusqlite::MAIN_DB,
                dir.join(format!(
                    "before-upgrade-{}.db",
                    chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
                )),
                None,
            )
            .map_err(err)?;
        }
        {
            let tx = conn.unchecked_transaction().map_err(err)?;
            let conn = &tx;
            conn.execute_batch(r#"CREATE TABLE IF NOT EXISTS cars(id INTEGER PRIMARY KEY,name TEXT NOT NULL,plate TEXT,vin TEXT,note TEXT); CREATE TABLE IF NOT EXISTS entries(id INTEGER PRIMARY KEY,car_id INTEGER NOT NULL REFERENCES cars(id),date TEXT NOT NULL,kind TEXT NOT NULL,category TEXT NOT NULL,cents INTEGER NOT NULL,party TEXT,note TEXT); CREATE TABLE IF NOT EXISTS categories(id INTEGER PRIMARY KEY,kind TEXT NOT NULL,name TEXT NOT NULL,active INTEGER NOT NULL DEFAULT 1,UNIQUE(kind,name));CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY);CREATE TABLE IF NOT EXISTS addresses(id INTEGER PRIMARY KEY,name TEXT NOT NULL,detail TEXT NOT NULL DEFAULT '',active INTEGER NOT NULL DEFAULT 1);CREATE UNIQUE INDEX IF NOT EXISTS addresses_active_name ON addresses(name) WHERE active=1;"#).map_err(err)?;
            for (table, col, typ) in [
                ("categories", "requires_route", "INTEGER NOT NULL DEFAULT 0"),
                ("addresses", "region_code", "TEXT"),
                (
                    "entries",
                    "start_address_id",
                    "INTEGER REFERENCES addresses(id)",
                ),
                (
                    "entries",
                    "end_address_id",
                    "INTEGER REFERENCES addresses(id)",
                ),
                ("entries", "start_address", "TEXT"),
                ("entries", "end_address", "TEXT"),
            ] {
                if rows(&conn, &format!("PRAGMA table_info({table})"))?
                    .iter()
                    .all(|r| r["name"] != col)
                {
                    conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {col} {typ};"))
                        .map_err(err)?;
                    if table == "categories" {
                        conn.execute("UPDATE categories SET requires_route=1 WHERE kind='收入' AND name='运费'",[]).map_err(err)?;
                    }
                }
            }
            if conn
                .query_row(
                    "SELECT 1 FROM settings WHERE key='categories_seeded'",
                    [],
                    |r| r.get::<_, i64>(0),
                )
                .optional()
                .map_err(err)?
                .is_none()
            {
                for (k, n) in [
                    ("收入", "运费"),
                    ("支出", "油费"),
                    ("支出", "通行费"),
                    ("支出", "倒板费"),
                    ("支出", "工资"),
                ] {
                    conn.execute(
                        "INSERT OR IGNORE INTO categories(kind,name,requires_route) VALUES(?,?,?)",
                        params![k, n, (n == "运费") as i64],
                    )
                    .map_err(err)?;
                }
                conn.execute("INSERT INTO settings VALUES('categories_seeded')", [])
                    .map_err(err)?;
            }
            tx.commit().map_err(err)?;
        }
        Ok(Self { conn, path })
    }
    pub fn data(&self) -> Result<Value> {
        let mut o = json!({});
        for t in ["cars", "entries", "categories", "addresses"] {
            o[t] = json!(rows(
                &self.conn,
                &format!("SELECT * FROM {t} ORDER BY id DESC")
            )?);
        }
        Ok(o)
    }
    pub fn export_rows(&self, ids: &[i64]) -> Result<Vec<Value>> {
        if ids.len() > 10000 {
            return Err("导出记录过多".into());
        }
        let all = rows(
            &self.conn,
            "SELECT e.*,c.name,c.plate FROM entries e JOIN cars c ON c.id=e.car_id",
        )?;
        ids.iter()
            .map(|id| {
                all.iter()
                    .find(|r| r["id"] == *id)
                    .cloned()
                    .ok_or_else(|| "记录不存在".into())
            })
            .collect()
    }
    pub fn snapshot_to(&self, path: impl AsRef<Path>) -> Result<()> {
        if path.as_ref() == self.path {
            return Err("备份路径不能与账本相同".into());
        }
        let path = path.as_ref();
        let temporary = tempfile::NamedTempFile::new_in(path.parent().unwrap_or(Path::new(".")))
            .map_err(err)?;
        self.conn
            .backup(rusqlite::MAIN_DB, temporary.path(), None)
            .map_err(err)?;
        temporary.persist(path).map_err(err)?;
        Ok(())
    }
    pub fn restore_from(&self, path: impl AsRef<Path>) -> Result<PathBuf> {
        let source = validated(path.as_ref())?;
        let tmp = tempfile::tempdir().map_err(err)?;
        let staged = tmp.path().join("restore.db");
        source
            .backup(rusqlite::MAIN_DB, &staged, None)
            .map_err(err)?;
        let prepared = Self::open(&staged)?;
        drop(prepared);
        let dir = self.path.parent().unwrap_or(Path::new(".")).join("backups");
        std::fs::create_dir_all(&dir).map_err(err)?;
        let safety = dir.join(format!(
            "before-restore-{}.db",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        self.snapshot_to(&safety)?;
        let mut dest = Connection::open(&self.path).map_err(err)?;
        dest.restore(
            rusqlite::MAIN_DB,
            &staged,
            None::<fn(rusqlite::backup::Progress)>,
        )
        .map_err(err)?;
        Ok(safety)
    }
    pub fn save(&self, resource: &str, d: Value) -> Result<Value> {
        let c = &self.conn;
        let resource = resource.trim_start_matches("/api/");
        let i = id(&d, "id")?;
        match resource {
            "cars" => {
                let plate = text(&d, "plate");
                if plate.is_empty() {
                    return Err("请填写车牌".into());
                }
                let name = if i.is_some() && d.get("name").is_none() {
                    c.query_row("SELECT name FROM cars WHERE id=?", [i], |r| {
                        r.get::<_, String>(0)
                    })
                    .map_err(|_| "车辆不存在".to_string())?
                } else {
                    text(&d, "name")
                };
                if let Some(i) = i {
                    c.execute(
                        "UPDATE cars SET name=?,plate=?,vin=?,note=? WHERE id=?",
                        params![name, plate, text(&d, "vin"), text(&d, "note"), i],
                    )
                } else {
                    c.execute(
                        "INSERT INTO cars(name,plate,vin,note) VALUES(?,?,?,?)",
                        params![name, plate, text(&d, "vin"), text(&d, "note")],
                    )
                }
                .map_err(err)?;
            }
            "categories/delete" => {
                c.execute("UPDATE categories SET active=0 WHERE id=?", [i])
                    .map_err(err)?;
            }
            "categories" => {
                let k = text(&d, "kind");
                let n = text(&d, "name");
                if !["收入", "支出"].contains(&k.as_str()) || n.is_empty() || n.chars().count() > 60
                {
                    return Err("类型无效".into());
                }
                let old = if let Some(i) = i {
                    Some(
                        c.query_row(
                            "SELECT requires_route FROM categories WHERE id=? AND active=1",
                            [i],
                            |r| r.get::<_, i64>(0),
                        )
                        .map_err(|_| "类型不存在".to_string())?,
                    )
                } else {
                    None
                };
                let route = d
                    .get("requires_route")
                    .map(|v| v.as_bool().unwrap_or(false) as i64)
                    .unwrap_or(old.unwrap_or(0));
                if let Some(i)=i{c.execute("UPDATE categories SET kind=?,name=?,requires_route=? WHERE id=?",params![k,n,route,i])}else{c.execute("INSERT INTO categories(kind,name,requires_route) VALUES(?,?,?) ON CONFLICT(kind,name) DO UPDATE SET active=1,requires_route=excluded.requires_route",params![k,n,route])}.map_err(err)?;
            }
            "addresses/delete" => {
                if c.execute("UPDATE addresses SET active=0 WHERE id=? AND active=1", [i])
                    .map_err(err)?
                    == 0
                {
                    return Err("地址已删除，请刷新".into());
                }
            }
            "addresses" => {
                let mut n = text(&d, "name");
                let mut detail = text(&d, "detail");
                let code = d
                    .get("region_code")
                    .filter(|v| !v.is_null())
                    .map(|_| text(&d, "region_code"));
                if let Some(code) = &code {
                    let r = crate::regions::lookup(code)
                        .filter(|r| r["level"].as_i64().unwrap_or(0) >= 2)
                        .ok_or("请选择城市、区县或乡镇/街道")?;
                    n = r["path"].as_str().unwrap().into();
                    detail.clear();
                }
                if n.is_empty() {
                    return Err("请填写地址名称".into());
                }
                if c.query_row(
                    "SELECT 1 FROM addresses WHERE active=1 AND name=? AND id!=?",
                    params![n, i.unwrap_or(0)],
                    |r| r.get::<_, i64>(0),
                )
                .optional()
                .map_err(err)?
                .is_some()
                {
                    return Err("地址名称已存在".into());
                }
                if let Some(i) = i {
                    if c.execute("UPDATE addresses SET name=?,detail=?,region_code=? WHERE id=? AND active=1",params![n,detail,code,i]).map_err(err)?==0{return Err("地址已删除，请刷新".into())}
                } else {
                    c.execute(
                        "INSERT INTO addresses(name,detail,region_code) VALUES(?,?,?)",
                        params![n, detail, code],
                    )
                    .map_err(err)?;
                }
            }
            "entries" => {
                let amount = cents(&d["amount"])?;
                let date = text(&d, "date");
                let parsed = chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d")
                    .map_err(|_| "请填写有效日期".to_string())?;
                if date.len() != 10
                    || parsed.format("%Y-%m-%d").to_string() != date
                    || date.starts_with("0000")
                {
                    return Err("请填写有效日期".into());
                }
                let k = text(&d, "kind");
                let cat = text(&d, "category");
                if !["收入", "支出"].contains(&k.as_str()) || cat.is_empty() {
                    return Err("请填写有效收支项目".into());
                }
                let old = if let Some(i) = i {
                    Some(
                        rows(c, &format!("SELECT * FROM entries WHERE id={i}"))?
                            .into_iter()
                            .next()
                            .ok_or("记录不存在，请刷新")?,
                    )
                } else {
                    None
                };
                let same = old
                    .as_ref()
                    .is_some_and(|o| o["kind"] == k && o["category"] == cat);
                let route=c.query_row("SELECT requires_route FROM categories WHERE kind=? AND name=? AND active=1",params![k,cat],|r|r.get::<_,i64>(0)).optional().map_err(err)?;
                if route.is_none() && !same {
                    return Err("请选择有效类型".into());
                }
                let start = id(&d, "start_address_id")?;
                let end = id(&d, "end_address_id")?;
                let legacy =
                    same && old.as_ref().is_some_and(|o| {
                        o["start_address_id"].is_null() && o["end_address_id"].is_null()
                    }) && start.is_none()
                        && end.is_none();
                if start.is_some() != end.is_some()
                    || (route == Some(1) && start.is_none() && !legacy)
                {
                    return Err("请同时选择起点和终点".into());
                }
                let mut snapshots = Vec::new();
                for (side, a) in [("start", start), ("end", end)] {
                    let snap = if let Some(o) = old
                        .as_ref()
                        .filter(|o| o[format!("{side}_address_id")] == json!(a))
                    {
                        o[format!("{side}_address")].as_str().map(str::to_owned)
                    } else if let Some(a) = a {
                        Some(
                            c.query_row(
                                "SELECT name,detail FROM addresses WHERE id=? AND active=1",
                                [a],
                                |r| {
                                    let n: String = r.get(0)?;
                                    let detail: String = r.get(1)?;
                                    Ok(if detail.is_empty() {
                                        n
                                    } else {
                                        format!("{n} / {detail}")
                                    })
                                },
                            )
                            .map_err(|_| "所选地址已删除，请刷新后重新选择".to_string())?,
                        )
                    } else {
                        None
                    };
                    snapshots.push(snap);
                }
                let car = id(&d, "car_id")?.ok_or("请选择车辆")?;
                if c.query_row("SELECT id FROM cars WHERE id=?", [car], |r| {
                    r.get::<_, i64>(0)
                })
                .optional()
                .map_err(err)?
                .is_none()
                {
                    return Err("车辆不存在，请刷新".into());
                }
                let party = text(&d, "party");
                let note = text(&d, "note");
                if let Some(i) = i {
                    c.execute("UPDATE entries SET car_id=?,date=?,kind=?,category=?,cents=?,party=?,note=?,start_address_id=?,end_address_id=?,start_address=?,end_address=? WHERE id=?",params![car,date,k,cat,amount,party,note,start,end,snapshots[0],snapshots[1],i]).map_err(err)?;
                } else {
                    c.execute("INSERT INTO entries(car_id,date,kind,category,cents,party,note,start_address_id,end_address_id,start_address,end_address) VALUES(?,?,?,?,?,?,?,?,?,?,?)",params![car,date,k,cat,amount,party,note,start,end,snapshots[0],snapshots[1]]).map_err(err)?;
                }
            }
            _ => return Err("不存在".into()),
        }
        Ok(json!({"ok":true}))
    }
}
fn validated(path: &Path) -> Result<Connection> {
    validated_inner(path, true)
}
fn validated_inner(path: &Path, limit: bool) -> Result<Connection> {
    let size = std::fs::metadata(path).map_err(err)?.len();
    if size == 0 || (limit && size > 50 * 1024 * 1024) {
        return Err("备份文件不能为空，最大50MB".into());
    }
    let c = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(err)?;
    if !rows(
        &c,
        "SELECT name FROM sqlite_master WHERE type IN ('trigger','view')",
    )?
    .is_empty()
    {
        return Err("备份包含不支持的视图或触发器".into());
    }
    let check: String = c
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .map_err(err)?;
    if check != "ok" {
        return Err("备份文件已损坏".into());
    }
    for (t, required) in [
        ("cars", vec!["id", "name", "plate", "vin", "note"]),
        (
            "entries",
            vec![
                "id", "car_id", "date", "kind", "category", "cents", "party", "note",
            ],
        ),
    ] {
        let cols = rows(&c, &format!("PRAGMA table_info({t})"))?;
        if required
            .iter()
            .any(|n| !cols.iter().any(|r| r["name"] == *n))
        {
            return Err("此文件不是兼容的运管家备份".into());
        }
    }
    if !rows(&c, "PRAGMA foreign_key_check")?.is_empty()
        || !rows(
            &c,
            "SELECT 1 FROM entries e LEFT JOIN cars c ON c.id=e.car_id WHERE c.id IS NULL LIMIT 1",
        )?
        .is_empty()
    {
        return Err("备份中存在无效数据关联".into());
    }
    Ok(c)
}
pub fn import_new(databasepath: impl AsRef<Path>, sourcepath: impl AsRef<Path>) -> Result<()> {
    let dest = databasepath.as_ref();
    if dest.exists() {
        return Err("目标账本已存在".into());
    }
    let source = validated(sourcepath.as_ref())?;
    let tmp =
        tempfile::NamedTempFile::new_in(dest.parent().unwrap_or(Path::new("."))).map_err(err)?;
    source
        .backup(rusqlite::MAIN_DB, tmp.path(), None)
        .map_err(err)?;
    drop(Ledger::open(tmp.path())?);
    tmp.persist_noclobber(dest).map_err(err)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn amounts() {
        for (s, n) in [
            ("12.34", 1234),
            ("1.2300", 123),
            ("1e2", 10000),
            (".01", 1),
            ("999999999", 99999999900),
        ] {
            assert_eq!(cents(&json!(s)).unwrap(), n)
        }
        for s in [
            "NaN",
            "Infinity",
            "0",
            "-1",
            "1.001",
            "999999999.01",
            "1e999999",
        ] {
            assert!(cents(&json!(s)).is_err(), "{s}")
        }
    }
    #[test]
    fn lifecycle() {
        let t = tempfile::tempdir().unwrap();
        let l = Ledger::open(t.path().join("db")).unwrap();
        l.save("cars", json!({"plate":"A","name":"old"})).unwrap();
        l.save("cars", json!({"id":1,"plate":"B"})).unwrap();
        assert_eq!(l.data().unwrap()["cars"][0]["name"], "old");
        let e = json!({"car_id":1,"date":"2026-10-31","kind":"支出","category":"油费","amount":"12.34"});
        l.save("entries", e.clone()).unwrap();
        assert_eq!(l.export_rows(&[1, 1]).unwrap().len(), 2);
        let backup = t.path().join("backup");
        l.snapshot_to(&backup).unwrap();
        l.save("cars", json!({"id":1,"plate":"C"})).unwrap();
        let safety = l.restore_from(&backup).unwrap();
        assert_eq!(l.data().unwrap()["cars"][0]["plate"], "B");
        assert_eq!(
            Ledger::open(safety).unwrap().data().unwrap()["cars"][0]["plate"],
            "C"
        );
        assert!(l.restore_from(t.path().join("missing")).is_err());
        assert!(!t.path().join("missing").exists());
    }
}
#[cfg(test)]
mod regression_tests {
    use super::*;
    #[test]
    fn route_snapshots_and_archived_category_survive_edit() {
        let t = tempfile::tempdir().unwrap();
        let l = Ledger::open(t.path().join("db")).unwrap();
        l.save("cars", json!({"plate":"A"})).unwrap();
        for n in ["上海", "杭州"] {
            l.save("addresses", json!({"name":n,"detail":"详细"}))
                .unwrap();
        }
        let mut e =
            json!({"car_id":1,"date":"2026-10-31","kind":"收入","category":"运费","amount":"1"});
        assert!(l.save("entries", e.clone()).is_err());
        e["start_address_id"] = json!(1);
        e["end_address_id"] = json!(2);
        l.save("entries", e.clone()).unwrap();
        l.save("addresses", json!({"id":1,"name":"新上海"}))
            .unwrap();
        l.save("addresses/delete", json!({"id":2})).unwrap();
        assert!(l.save("entries", e.clone()).is_err());
        l.save("categories/delete", json!({"id":1})).unwrap();
        e["id"] = json!(1);
        e["amount"] = json!("2.50");
        l.save("entries", e.clone()).unwrap();
        let d = l.data().unwrap();
        assert_eq!(d["entries"][0]["start_address"], "上海 / 详细");
        assert_eq!(d["entries"][0]["end_address"], "杭州 / 详细");
        assert_eq!(d["entries"][0]["cents"], 250);
        e["amount"] = json!("1.001");
        assert!(l.save("entries", e).is_err());
        assert_eq!(l.data().unwrap()["entries"][0]["cents"], 250);
    }
    #[test]
    fn import_does_not_modify_source_or_overwrite_target() {
        let t = tempfile::tempdir().unwrap();
        let source = t.path().join("source");
        let l = Ledger::open(&source).unwrap();
        l.save("cars", json!({"plate":"original"})).unwrap();
        drop(l);
        let before = std::fs::read(&source).unwrap();
        let dest = t.path().join("new");
        import_new(&dest, &source).unwrap();
        assert_eq!(before, std::fs::read(&source).unwrap());
        assert!(import_new(&dest, &source).is_err());
        assert_eq!(
            Ledger::open(dest).unwrap().data().unwrap()["cars"][0]["plate"],
            "original"
        );
    }
    #[test]
    fn invalid_restore_keeps_live_database() {
        let t = tempfile::tempdir().unwrap();
        let l = Ledger::open(t.path().join("live")).unwrap();
        let bad = t.path().join("bad");
        Connection::open(&bad)
            .unwrap()
            .execute_batch("CREATE TABLE unrelated(x);")
            .unwrap();
        let before = l.data().unwrap();
        assert!(l.restore_from(&bad).is_err());
        assert_eq!(before, l.data().unwrap());
        assert!(!t.path().join("backups").exists());
        assert!(Ledger::open(&bad).is_err());
    }
}

#[cfg(test)]
mod migration_tests {
    use super::*;
    #[test]
    fn legacy_upgrade_preserves_ids_amounts_and_creates_one_backup() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("legacy.db");
        let old = Connection::open(&path).unwrap();
        old.execute_batch("CREATE TABLE cars(id INTEGER PRIMARY KEY,name TEXT NOT NULL,plate TEXT,vin TEXT,note TEXT); CREATE TABLE entries(id INTEGER PRIMARY KEY,car_id INTEGER,date TEXT,kind TEXT,category TEXT,cents INTEGER,party TEXT,note TEXT); INSERT INTO cars VALUES(42,'历史车型','旧车牌','VIN','备注'); INSERT INTO entries VALUES(9,42,'2026-10-31','收入','运费',12345,'客户','旧备注');").unwrap();
        drop(old);
        for _ in 0..2 {
            let ledger = Ledger::open(&path).unwrap();
            let data = ledger.data().unwrap();
            assert_eq!(data["cars"][0]["id"], 42);
            assert_eq!(data["entries"][0]["id"], 9);
            assert_eq!(data["entries"][0]["cents"], 12345);
            assert_eq!(data["entries"][0]["car_id"], 42);
            assert!(data["entries"][0]["start_address"].is_null());
            // Historical freight without a route can still be edited.
            ledger.save("entries",json!({"id":9,"car_id":42,"date":"2026-10-31","kind":"收入","category":"运费","amount":"123.45","note":"旧备注"})).unwrap();
            // Legacy schemas without foreign keys still reject nonexistent cars.
            assert!(ledger.save("entries",json!({"car_id":999,"date":"2026-10-31","kind":"支出","category":"工资","amount":"1"})).is_err());
        }
        assert_eq!(
            std::fs::read_dir(folder.path().join("backups"))
                .unwrap()
                .count(),
            1
        );
    }
    #[test]
    fn renamed_freight_keeps_route_requirement_and_allows_same_endpoint() {
        let folder = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(folder.path().join("test.db")).unwrap();
        ledger.save("cars", json!({"plate":"测试"})).unwrap();
        ledger
            .save(
                "categories",
                json!({"id":1,"kind":"收入","name":"运输收入"}),
            )
            .unwrap();
        let mut entry = json!({"car_id":1,"kind":"收入","category":"运输收入","date":"2026-10-31","amount":"10"});
        assert!(ledger.save("entries", entry.clone()).is_err());
        ledger.save("addresses", json!({"name":"同地点"})).unwrap();
        entry["start_address_id"] = json!(1);
        entry["end_address_id"] = json!(1);
        ledger.save("entries", entry).unwrap();
        let data = ledger.data().unwrap();
        assert_eq!(
            data["entries"][0]["start_address"],
            data["entries"][0]["end_address"]
        );
    }
}
