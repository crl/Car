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
fn place_reading(name: &str) -> Option<(&'static str, &'static str)> {
    Some(match name {
        "重庆市" => ("chongqingshi", "cqs"),
        "长沙市" => ("changshashi", "css"),
        "长春市" => ("changchunshi", "ccs"),
        "长治市" => ("changzhishi", "czs"),
        "吕梁市" => ("lvliangshi", "lls"),
        "长安区" => ("changanqu", "caq"),
        "厦门市" => ("xiamenshi", "xms"),
        "六安市" => ("luanshi", "las"),
        "蚌埠市" => ("bengbushi", "bbs"),
        "铅山县" => ("yanshanxian", "ysx"),
        "都江堰市" => ("dujiangyanshi", "djys"),
        "蔚县" => ("yuxian", "yx"),
        "单县" => ("shanxian", "sx"),
        "什邡市" => ("shifangshi", "sfs"),
        _ => return None,
    })
}

#[derive(Default)]
struct PinyinKeys {
    full: String,
    initials: String,
}

impl PinyinKeys {
    fn append(&mut self, text: &str) {
        for ch in text.chars() {
            if let Some(py) = ch.to_pinyin() {
                self.full.extend(
                    py.plain()
                        .chars()
                        .map(|ch| if ch == 'ü' { 'v' } else { ch }),
                );
                if let Some(first) = py.plain().chars().next() {
                    self.initials.push(first);
                }
            } else if ch.is_ascii_alphanumeric() {
                self.full.push(ch.to_ascii_lowercase());
                self.initials.push(ch.to_ascii_lowercase());
            }
        }
    }

    fn contains(&self, word: &str) -> bool {
        self.full.contains(word) || self.initials.contains(word)
    }
}

fn search_keys() -> &'static [(PinyinKeys, Option<PinyinKeys>)] {
    static KEYS: OnceLock<Vec<(PinyinKeys, Option<PinyinKeys>)>> = OnceLock::new();
    KEYS.get_or_init(|| {
        index()
            .iter()
            .map(|region| {
                let path = region["path"].as_str().unwrap();
                let mut keys = PinyinKeys::default();
                keys.append(path);
                let alternate = path
                    .split(" / ")
                    .any(|name| place_reading(name).is_some())
                    .then(|| {
                        let mut keys = PinyinKeys::default();
                        for name in path.split(" / ") {
                            if let Some((full, initials)) = place_reading(name) {
                                keys.full.push_str(full);
                                keys.initials.push_str(initials);
                            } else {
                                keys.append(name);
                            }
                        }
                        keys
                    });
                (keys, alternate)
            })
            .collect()
    })
}

fn place_priority(path: &str, query: &str) -> usize {
    // Prefer matches to the corrected name itself, then its nearest administrative descendants.
    path.rsplit(" / ")
        .position(|name| {
            let Some((full, initials)) = place_reading(name) else {
                return false;
            };
            let (suffix, suffix_initial) = match name.chars().last() {
                Some('市') => ("shi", "s"),
                Some('区') => ("qu", "q"),
                Some('县') => ("xian", "x"),
                _ => ("", ""),
            };
            [
                full,
                initials,
                full.strip_suffix(suffix).unwrap_or(full),
                initials.strip_suffix(suffix_initial).unwrap_or(initials),
            ]
            .contains(&query)
        })
        .unwrap_or(usize::MAX)
}

pub fn query(parent: &str, q: &str) -> Vec<Value> {
    query_with_limit(parent, q, 100)
}
pub fn query_with_limit(parent: &str, q: &str, limit: usize) -> Vec<Value> {
    let normalized = q.to_lowercase().replace('ü', "v");
    let words: Vec<_> = normalized.split_whitespace().collect();
    let keys = if words.is_empty() {
        None
    } else {
        Some(search_keys())
    };
    let matches = index()
        .iter()
        .enumerate()
        .filter(|(i, r)| {
            if words.is_empty() {
                r["parent"] == parent
            } else {
                r["level"].as_u64().unwrap() >= 2
                    && words.iter().all(|w| {
                        let (original, alternate) = &keys.unwrap()[*i];
                        r["path"].as_str().unwrap().contains(w)
                            || original.contains(w)
                            || alternate.as_ref().is_some_and(|keys| keys.contains(w))
                    })
            }
        })
        .map(|(_, region)| region);
    if let [word] = words.as_slice() {
        if limit != usize::MAX && word.bytes().all(|byte| byte.is_ascii_lowercase()) {
            let mut matches: Vec<_> = matches.collect();
            matches.sort_by_cached_key(|region| {
                place_priority(region["path"].as_str().unwrap(), word)
            });
            return matches.into_iter().take(limit).cloned().collect();
        }
    }
    matches
        .take(if words.is_empty() { usize::MAX } else { limit })
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
    fn pinyin_search_matches_full_initials_and_mixed_queries() {
        for q in ["xihu", "XI HU", "xh", "杭州 xihu", "zhejiang hangzhou xihu"] {
            assert!(
                query_with_limit("", q, usize::MAX)
                    .iter()
                    .any(|r| r["code"] == "330106"),
                "{q}"
            );
        }
        for q in ["chongqing 渝中", "重庆 yz", "cq yz"] {
            assert!(query("", q).iter().any(|r| r["code"] == "500103"), "{q}");
        }
    }
    #[test]
    fn common_polyphonic_places_can_be_found_within_the_display_limit() {
        for (code, full, initials) in [
            ("500103", "chongqing", "cq"),
            ("4301", "changsha", "cs"),
            ("2201", "changchun", "cc"),
            ("1404", "changzhi", "cz"),
            ("610116", "changan", "ca"),
            ("3502", "xiamen", "xm"),
            ("3415", "luan", "la"),
            ("3403", "bengbu", "bb"),
            ("361124", "yanshan", "ys"),
            ("510181", "dujiangyan", "djy"),
            ("130726", "yuxian", "yx"),
            ("371722", "shanxian", "sx"),
            ("510682", "shifang", "sf"),
            ("1411", "lvliang", "ll"),
        ] {
            for query_text in [full, initials] {
                assert!(
                    query("", query_text)
                        .iter()
                        .any(|region| region["code"] == code),
                    "{query_text}: {code}"
                );
            }
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
