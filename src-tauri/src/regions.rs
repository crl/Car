use pinyin::ToPinyin;
use serde_json::{json, Value};
use std::sync::OnceLock;
fn index() -> &'static Vec<Value> {
    static INDEX: OnceLock<Vec<Value>> = OnceLock::new();
    INDEX.get_or_init(|| {
  fn walk(nodes: &[Value], parent: &str, path: &str, level: u64, out: &mut Vec<Value>) {
   for n in nodes { let code=n["code"].as_str().unwrap(); let name=n["name"].as_str().unwrap(); let p=if path.is_empty(){name.to_owned()}else{format!("{path} / {name}")}; let children=n["children"].as_array();
    out.push(json!({"code":code,"name":name,"parent":parent,"level":level,"path":p,"has_children":children.is_some_and(|v|!v.is_empty())}));
    if let Some(c)=children {walk(c,code,&p,level+1,out)}
   }
  }
  let tree:Value=serde_json::from_str(include_str!("../../data/regions.json")).expect("bundled regions"); let mut out=Vec::new();walk(tree.as_array().unwrap(),"","",1,&mut out);out
 })
}
fn search_keys() -> &'static Vec<(String, String)> {
    static KEYS: OnceLock<Vec<(String, String)>> = OnceLock::new();
    KEYS.get_or_init(|| index().iter().map(|region| {
        let mut full = String::new();
        let mut initials = String::new();
        for ch in region["path"].as_str().unwrap().chars() {
            if let Some(py) = ch.to_pinyin() {
                full.push_str(py.plain());
                if let Some(first) = py.plain().chars().next() { initials.push(first); }
            } else if ch.is_ascii_alphanumeric() {
                full.push(ch.to_ascii_lowercase());
                initials.push(ch.to_ascii_lowercase());
            }
        }
        (full, initials)
    }).collect())
}
pub fn query(parent: &str, q: &str) -> Vec<Value> {
    query_with_limit(parent, q, 100)
}
pub fn query_with_limit(parent: &str, q: &str, limit: usize) -> Vec<Value> {
    let normalized = q.to_lowercase();
    let words: Vec<_> = normalized.split_whitespace().collect();
    let keys = if words.is_empty() { None } else { Some(search_keys()) };
    index()
        .iter()
        .enumerate()
        .filter(|(i, r)| {
            if words.is_empty() {
                r["parent"] == parent
            } else {
                r["level"].as_u64().unwrap() >= 2
                    && words
                        .iter()
                        .all(|w| {
                            let (full, initials) = &keys.unwrap()[*i];
                            r["path"].as_str().unwrap().contains(w) || full.contains(w) || initials.contains(w)
                        })
            }
        })
        .take(if words.is_empty() { usize::MAX } else { limit })
        .map(|(_, region)| region.clone())
        .collect()
}
pub fn lookup(code: &str) -> Option<Value> {
    index().iter().find(|r| r["code"] == code).cloned()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pinyin_search_matches_full_initials_and_mixed_queries() {
        for q in ["xihu", "XI HU", "xh", "杭州 xihu", "zhejiang hangzhou xihu"] {
            assert!(query_with_limit("", q, usize::MAX).iter().any(|r| r["code"] == "330106"), "{q}");
        }
    }
    #[test]
    fn unlimited_search_returns_all_matching_paths() {
        let expected: Vec<_> = index().iter().filter(|r| r["level"].as_u64().unwrap() >= 2 && r["path"].as_str().unwrap().contains("西湖")).cloned().collect();
        assert!(!expected.is_empty());
        assert_eq!(query_with_limit("", "西湖", usize::MAX), expected);
        assert!(query_with_limit("", "街道", usize::MAX).len() > 100);
        assert_eq!(query("", "街道").len(), 100);
    }
    #[test]
    fn regions() {
        assert_eq!(query("", "").len(), 31);
        assert!(!query("", "浙江 杭州").is_empty());
        assert!(lookup("invalid").is_none());
    }
}
