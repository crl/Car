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
pub fn query(parent: &str, q: &str) -> Vec<Value> {
    let words: Vec<_> = q.split_whitespace().collect();
    index()
        .iter()
        .filter(|r| {
            if words.is_empty() {
                r["parent"] == parent
            } else {
                r["level"].as_u64().unwrap() >= 2
                    && words
                        .iter()
                        .all(|w| r["path"].as_str().unwrap().contains(w))
            }
        })
        .take(if words.is_empty() { usize::MAX } else { 100 })
        .cloned()
        .collect()
}
pub fn lookup(code: &str) -> Option<Value> {
    index().iter().find(|r| r["code"] == code).cloned()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn regions() {
        assert_eq!(query("", "").len(), 31);
        assert!(!query("", "浙江 杭州").is_empty());
        assert!(lookup("invalid").is_none());
    }
}
