//! Compares `Utils` and `path.posix` with `vscode-uri` 3.2.0. Fixture:
//! `scripts/css/trace-uri.mjs`.

use serde_json::{Value, json};
use vscode_uri::{Uri, basename, dirname, join_path, posix, resolve_path};

#[test]
fn upstream() {
    let f: Value = serde_json::from_str(include_str!("fixtures/utils.json")).unwrap();
    let pairs = |name: &str| f[name].as_array().unwrap().clone();
    for c in pairs("normalize") {
        assert_eq!(posix::normalize(c[0].as_str().unwrap()), c[1], "{c}");
    }
    for c in pairs("dirname") {
        assert_eq!(posix::dirname(c[0].as_str().unwrap()), c[1], "{c}");
    }
    for c in pairs("basename") {
        assert_eq!(posix::basename(c[0].as_str().unwrap()), c[1], "{c}");
    }
    for c in pairs("join") {
        let args: Vec<&str> = c[0]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a.as_str().unwrap())
            .collect();
        assert_eq!(posix::join(&args), c[1], "{c}");
    }
    let s = |u: Option<Uri>| u.map(|u| u.to_string_unencoded());
    for c in pairs("uris") {
        let uri = Uri::parse(c["uri"].as_str().unwrap());
        assert_eq!(json!(s(uri.clone())), c["string"], "{c}");
        let Some(uri) = uri else { continue };
        assert_eq!(json!(s(dirname(&uri))), c["dirname"], "{c}");
        assert_eq!(basename(&uri), c["basename"], "{c}");
        let joins = [
            "",
            "node_modules",
            "package.json",
            "../x",
            "/abs",
            "a b",
            "ä#?.css",
            "..",
            ".",
            "./_index.scss",
            "../../..",
        ];
        for (j, expected) in joins.iter().zip(c["join"].as_array().unwrap()) {
            assert_eq!(json!(s(join_path(&uri, &[j]))), *expected, "{c} {j}");
        }
        let join2 = join_path(&uri, &["node_modules", "@a/b", "package.json"]);
        assert_eq!(json!(s(join2)), c["join2"], "{c}");
        for r in c["resolve"].as_array().unwrap() {
            let path = r[0].as_str().unwrap();
            assert_eq!(json!(s(resolve_path(&uri, path))), r[1], "{c} {path}");
        }
    }
    for c in pairs("files") {
        let uri = Uri::file(c[0].as_str().unwrap());
        assert_eq!(uri.to_string_unencoded(), c[1], "{c}");
        assert_eq!(uri.to_string_encoded(), c[2], "{c}");
    }
}
