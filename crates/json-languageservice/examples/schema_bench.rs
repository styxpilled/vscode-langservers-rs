//! Times schema load, validation and completion for a document with a `file:` schema, and
//! counts heap bytes.
//! Usage: `schema_bench <document path>`

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use json_languageservice::{
    LanguageService, LanguageServiceParams, LanguageSettings, SchemaRequestError, TextDocument,
    parse,
};
use lsp_types::Position;

struct Counting;
static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let n = CURRENT.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
        PEAK.fetch_max(n, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        CURRENT.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

fn mb(n: usize) -> String {
    format!("{:.1} MB", n as f64 / 1048576.0)
}

fn heap(label: &str) {
    println!(
        "  {label}: heap {}, peak {}",
        mb(CURRENT.load(Ordering::Relaxed)),
        mb(PEAK.load(Ordering::Relaxed))
    );
    PEAK.store(CURRENT.load(Ordering::Relaxed), Ordering::Relaxed);
}

fn main() {
    let path = std::env::args().nth(1).expect("document path");
    let text = std::fs::read_to_string(&path).unwrap();
    let uri = url::Url::from_file_path(std::fs::canonicalize(&path).unwrap()).unwrap();
    let mut ls = LanguageService::new(LanguageServiceParams {
        schema_request_service: Some(Box::new(|uri| {
            let path = url::Url::parse(uri).unwrap().to_file_path().unwrap();
            std::fs::read_to_string(path).map_err(|e| SchemaRequestError {
                message: e.to_string(),
                code: None,
            })
        })),
        ..Default::default()
    });
    let doc = TextDocument::new(uri.as_str().parse().unwrap(), "json", 1, text);
    let json = parse(&doc, true);
    heap("start");
    for _ in 0..3 {
        let t = Instant::now();
        ls.configure(&LanguageSettings::default());
        println!("configure {:?}", t.elapsed());
        heap("configure");
        let t = Instant::now();
        let diagnostics = ls.do_validation(&doc, &json, None, None);
        let validate = t.elapsed();
        heap("validate");
        let t = Instant::now();
        let list = ls.do_complete(&doc, Position::new(1, 0), &json);
        let complete = t.elapsed();
        heap("complete");
        let t = Instant::now();
        let value = serde_json::to_value(&list).unwrap();
        let to_value = t.elapsed();
        let t = Instant::now();
        let s = serde_json::to_string(&value).unwrap();
        let to_string = t.elapsed();
        heap("serialize");
        drop((list, value, s));
        println!(
            "validate (with load) {validate:?} ({} diagnostics), complete {complete:?}, to_value {to_value:?}, to_string {to_string:?}",
            diagnostics.len()
        );
    }
}
