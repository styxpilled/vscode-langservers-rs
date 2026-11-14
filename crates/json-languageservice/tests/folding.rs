//! Port of `vscode-json-languageservice/src/test/folding.test.ts`.

mod common;

use json_languageservice::get_folding_ranges;
use lsp_types::FoldingRangeKind;

fn assert_ranges(
    lines: &[&str],
    expected: &[(u32, u32, Option<FoldingRangeKind>)],
    range_limit: Option<usize>,
) {
    let document = common::document("test://foo/bar.json", &lines.join("\n"));
    let mut actual: Vec<_> = get_folding_ranges(&document, range_limit)
        .into_iter()
        .map(|r| (r.start_line, r.end_line, r.kind))
        .collect();
    actual.sort_by_key(|r| r.0);
    assert_eq!(actual, expected);
}

#[test]
fn fold_one_level() {
    assert_ranges(&["{", "\"foo\":\"bar\"", "}"], &[(0, 1, None)], None);
}

#[test]
fn fold_two_level() {
    assert_ranges(
        &["[", "{", "\"foo\":\"bar\"", "}", "]"],
        &[(0, 3, None), (1, 2, None)],
        None,
    );
}

#[test]
fn fold_arrays() {
    assert_ranges(
        &["[", "[", "],[", "1", "]", "]"],
        &[(0, 4, None), (2, 3, None)],
        None,
    );
}

#[test]
fn filter_start_on_same_line() {
    assert_ranges(
        &["[[", "[", "],[", "1", "]", "]]"],
        &[(0, 4, None), (2, 3, None)],
        None,
    );
}

#[test]
fn fold_commment() {
    assert_ranges(
        &["/*", " multi line", "*/"],
        &[(0, 2, Some(FoldingRangeKind::Comment))],
        None,
    );
}

#[test]
fn incomplete_commment() {
    assert_ranges(&["/*", "{", "\"foo\":\"bar\"", "}"], &[(1, 2, None)], None);
}

#[test]
fn fold_regions() {
    assert_ranges(
        &["// #region", "{", "}", "// #endregion"],
        &[(0, 3, Some(FoldingRangeKind::Region))],
        None,
    );
}

#[test]
fn test_limit() {
    assert_ranges(
        &[
            "[", " [", "  [", "  ", "  ],", "  [", "   [", "  ", "   ],", "   [", "  ", "   ],",
            "  ],", "  [", "  ", "  ],", "  [", "  ", "  ]", " ]", "]",
        ],
        &[
            (0, 19, None),
            (1, 18, None),
            (2, 3, None),
            (5, 11, None),
            (6, 7, None),
            (9, 10, None),
            (13, 14, None),
            (16, 17, None),
        ],
        None,
    );
    assert_ranges(
        &[
            "[", " [", "  [", "  ", "  ],", "  [", "   [", "  ", "   ],", "   [", "  ", "   ],",
            "  ],", "  [", "  ", "  ],", "  [", "  ", "  ]", " ]", "]",
        ],
        &[
            (0, 19, None),
            (1, 18, None),
            (2, 3, None),
            (5, 11, None),
            (6, 7, None),
            (9, 10, None),
            (13, 14, None),
            (16, 17, None),
        ],
        Some(8),
    );
    assert_ranges(
        &[
            "[", " [", "  [", "  ", "  ],", "  [", "   [", "  ", "   ],", "   [", "  ", "   ],",
            "  ],", "  [", "  ", "  ],", "  [", "  ", "  ]", " ]", "]",
        ],
        &[
            (0, 19, None),
            (1, 18, None),
            (2, 3, None),
            (5, 11, None),
            (6, 7, None),
            (13, 14, None),
            (16, 17, None),
        ],
        Some(7),
    );
    assert_ranges(
        &[
            "[", " [", "  [", "  ", "  ],", "  [", "   [", "  ", "   ],", "   [", "  ", "   ],",
            "  ],", "  [", "  ", "  ],", "  [", "  ", "  ]", " ]", "]",
        ],
        &[
            (0, 19, None),
            (1, 18, None),
            (2, 3, None),
            (5, 11, None),
            (13, 14, None),
            (16, 17, None),
        ],
        Some(6),
    );
    assert_ranges(
        &[
            "[", " [", "  [", "  ", "  ],", "  [", "   [", "  ", "   ],", "   [", "  ", "   ],",
            "  ],", "  [", "  ", "  ],", "  [", "  ", "  ]", " ]", "]",
        ],
        &[
            (0, 19, None),
            (1, 18, None),
            (2, 3, None),
            (5, 11, None),
            (13, 14, None),
        ],
        Some(5),
    );
    assert_ranges(
        &[
            "[", " [", "  [", "  ", "  ],", "  [", "   [", "  ", "   ],", "   [", "  ", "   ],",
            "  ],", "  [", "  ", "  ],", "  [", "  ", "  ]", " ]", "]",
        ],
        &[(0, 19, None), (1, 18, None), (2, 3, None), (5, 11, None)],
        Some(4),
    );
    assert_ranges(
        &[
            "[", " [", "  [", "  ", "  ],", "  [", "   [", "  ", "   ],", "   [", "  ", "   ],",
            "  ],", "  [", "  ", "  ],", "  [", "  ", "  ]", " ]", "]",
        ],
        &[(0, 19, None), (1, 18, None), (2, 3, None)],
        Some(3),
    );
    assert_ranges(
        &[
            "[", " [", "  [", "  ", "  ],", "  [", "   [", "  ", "   ],", "   [", "  ", "   ],",
            "  ],", "  [", "  ", "  ],", "  [", "  ", "  ]", " ]", "]",
        ],
        &[(0, 19, None), (1, 18, None)],
        Some(2),
    );
    assert_ranges(
        &[
            "[", " [", "  [", "  ", "  ],", "  [", "   [", "  ", "   ],", "   [", "  ", "   ],",
            "  ],", "  [", "  ", "  ],", "  [", "  ", "  ]", " ]", "]",
        ],
        &[(0, 19, None)],
        Some(1),
    );
}
