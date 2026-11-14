//! Port of `vscode-json-languageservice/src/test/sort.test.ts`.

mod common;

use json_languageservice::{FormattingOptions, sort};

fn test_sort(unsorted: &str, expected: &str) {
    let document = common::document("test://test.json", unsorted);
    let options = FormattingOptions {
        tab_size: 2,
        insert_spaces: true,
        ..Default::default()
    };
    let edits = sort(&document, &options);
    assert_eq!(document.apply_edits(&edits), expected);
}

#[test]
fn sorting_a_simple_jsonc_object_with_numeric_values() {
    test_sort("{\"b\" : 1, \"a\" : 2}", "{\n  \"a\": 2,\n  \"b\": 1\n}");
}

#[test]
fn sorting_a_simple_jsonc_object_with_an_array_spanning_several_lines() {
    test_sort(
        "{\"array\":[\"volleyball\",\n      \"drawing\",\n  \"hiking\"]}",
        "{\n  \"array\": [\n    \"volleyball\",\n    \"drawing\",\n    \"hiking\"\n  ]\n}",
    );
}

#[test]
fn sorting_a_jsonc_object_with_nested_objects() {
    test_sort(
        "{\"name\": \"Brigitte\",\"age\" : 30,\n\"hobbies\" : [\"volleyball\",\"drawing\",\"hiking\"],\n\"friends\" : {\n\"Marc\" : {\"hobbies\" : [\"kayaking\", \"mountaineering\"],\n\"age\" : 35,},\n\"Leila\" : {\"hobbies\" : [\"watching movies\",\n\"reading books\"], \"age\" : 32}}}",
        "{\n  \"age\": 30,\n  \"friends\": {\n    \"Leila\": {\n      \"age\": 32,\n      \"hobbies\": [\n        \"watching movies\",\n        \"reading books\"\n      ]\n    },\n    \"Marc\": {\n      \"age\": 35,\n      \"hobbies\": [\n        \"kayaking\",\n        \"mountaineering\"\n      ]\n    }\n  },\n  \"hobbies\": [\n    \"volleyball\",\n    \"drawing\",\n    \"hiking\"\n  ],\n  \"name\": \"Brigitte\"\n}",
    );
}

#[test]
fn sorting_a_jsonc_object_with_line_comments() {
    test_sort(
        "{ // this is a comment\n\"boolean\" : true,\n\"array\" : [\n// this is a second comment\n \"element1\", \"element2\"]\n}",
        "{ // this is a comment\n  \"array\": [\n    // this is a second comment\n    \"element1\",\n    \"element2\"\n  ],\n  \"boolean\": true\n}",
    );
}

#[test]
fn sorting_a_jsonc_object_with_an_object_nested_inside_of_an_array_value() {
    test_sort(
        "{\n\"boolean\" : true,\n\"array\" : [\n \"element1\", {\"property\" : \"element2\"}, \"element3\"]\n}",
        "{\n  \"array\": [\n    \"element1\",\n    {\n      \"property\": \"element2\"\n    },\n    \"element3\"\n  ],\n  \"boolean\": true\n}",
    );
}

#[test]
fn sorting_a_jsonc_object_with_comments_appearing_before_and_after_the_main_json_object() {
    test_sort(
        "// comment appearing before\n\n{\n\"boolean\" : true,\n\"array\" : [\n \"element1\", {\"property\" : \"element2\"}, \"element3\"]\n} /* block comment appearing \nafter, it spans several\nlines */",
        "// comment appearing before\n{\n  \"array\": [\n    \"element1\",\n    {\n      \"property\": \"element2\"\n    },\n    \"element3\"\n  ],\n  \"boolean\": true\n} /* block comment appearing \nafter, it spans several\nlines */",
    );
}

#[test]
fn sorting_a_jsonc_object_with_new_lines_appearing_before_and_after_the_main_json_object() {
    test_sort(
        "\n\n{\n\"boolean\" : true,\n\"array\" : [\n \"element1\", {\"property\" : \"element2\"}, \"element3\"]\n}\n\n",
        "{\n  \"array\": [\n    \"element1\",\n    {\n      \"property\": \"element2\"\n    },\n    \"element3\"\n  ],\n  \"boolean\": true\n}",
    );
}

#[test]
fn sorting_a_jsonc_object_with_a_block_comment_appearing_on_the_same_line_as_a_comma_but_not_ending_on_that_line()
 {
    test_sort(
        "{\n\"boolean\" : true, /* this is block comment starting on\nthe line where the comma is but ending on another line */\n\"array\" : []\n}",
        "{\n  \"array\": [],\n  \"boolean\": true /* this is block comment starting on\nthe line where the comma is but ending on another line */\n}",
    );
}

#[test]
fn sorting_a_jsonc_object_with_a_block_comment_starting_at_the_end_of_a_property_and_such_that_a_new_property_starts_on_the_end_of_that_block_comment()
 {
    test_sort(
        "{\n\"boolean\" : true, /* this is block comment starting on\nthe line where the comma is but ending on another line */ \"array\" : []\n}",
        "{\n  \"array\": [],\n  \"boolean\": true /* this is block comment starting on\nthe line where the comma is but ending on another line */\n}",
    );
}

#[test]
fn sorting_a_jsonc_object_with_comments_between_properties() {
    test_sort(
        "// comment appearing before\n\n{\n // some comment\n\"boolean\" : true,\n // some other comment\n\"numeric\" : 2,\n /* a third comment\n which is a block comment */\n\"array\": []\n}",
        "// comment appearing before\n{\n  /* a third comment\n which is a block comment */\n  \"array\": [],\n  // some comment\n  \"boolean\": true,\n  // some other comment\n  \"numeric\": 2\n}",
    );
}

#[test]
fn sorting_a_jsonc_object_with_comments_appearing_between_a_value_and_the_comma() {
    test_sort(
        "{\n\"boolean\" : true // some comment\n,\n\"array\" : [],\n\"numeric\" : 2\n}",
        "{\n  \"array\": [],\n  \"boolean\": true // some comment\n  ,\n  \"numeric\": 2\n}",
    );
}

#[test]
fn sorting_a_jsonc_object_with_block_comments_on_the_same_line_as_the_arrays() {
    test_sort(
        "/* multi-line comment\n..\n*/\n{ /* multi-line comment\n..\n*/ \n\"information_for_contributors\": [\n/* multi-line comment\n..\n*/\n\"This file has been converted from https://github.com/textmate/perl.tmbundle/blob/master/Syntaxes/Perl%206.tmLanguage\",\n\"Once accepted there, we are happy to receive an update request.\"\n], /* multi-line comment\n..\n*/\n\"version\": \"https://github.com/textmate/perl.tmbundle/commit/d9841a0878239fa43f88c640f8d458590f97e8f5\", /* multi-line comment\n..\n*/\n\"name\": \"Perl 6\" /* multi-line comment\n..\n*/\n}",
        "/* multi-line comment\n..\n*/\n{ /* multi-line comment\n..\n*/\n  \"information_for_contributors\": [\n    /* multi-line comment\n..\n*/\n    \"This file has been converted from https://github.com/textmate/perl.tmbundle/blob/master/Syntaxes/Perl%206.tmLanguage\",\n    \"Once accepted there, we are happy to receive an update request.\"\n  ], /* multi-line comment\n..\n*/\n  \"name\": \"Perl 6\", /* multi-line comment\n..\n*/\n  \"version\": \"https://github.com/textmate/perl.tmbundle/commit/d9841a0878239fa43f88c640f8d458590f97e8f5\" /* multi-line comment\n..\n*/\n}",
    );
}

#[test]
fn sorting_a_jsonc_object_where_the_colon_is_not_on_the_same_line_as_the_key_or_the_value() {
    test_sort(
        "{\n\"boolean\"\n:\ntrue // some comment\n,\n\"array\"\n: [],\n\"numeric\" : 2\n}",
        "{\n  \"array\": [],\n  \"boolean\": true // some comment\n  ,\n  \"numeric\": 2\n}",
    );
}

#[test]
fn sorting_a_complicated_jsonc_object_1() {
    test_sort(
        "// Comment ouside the main JSON object\n\n{\n// A comment which belongs to b\n\"b\": \"some value\",\n\n\"a\": \"some other value\" /* a block comment which starts on the same line as key a\n..*/,\n\n\"array\": [\n\"first element\",\n{\n    // comment belonging to r\n    \"r\" : 1,\n\n    // comment belonging to q\n    \"q\" : {\n        \"s\" : 2\n    },\n    // comment belonging to p\n    \"p\" : 3\n},\n\"third element\"\n] // some comment on the line where the array ends\n,\n\n\"numeric\" : [ 1, 2, 3]\n}\n\n\n/* Comment below the main JSON object\n...\n...\n*/",
        "// Comment ouside the main JSON object\n{\n  \"a\": \"some other value\" /* a block comment which starts on the same line as key a\n..*/,\n  \"array\": [\n    \"first element\",\n    {\n      // comment belonging to p\n      \"p\": 3,\n      // comment belonging to q\n      \"q\": {\n        \"s\": 2\n      },\n      // comment belonging to r\n      \"r\": 1\n    },\n    \"third element\"\n  ] // some comment on the line where the array ends\n  ,\n  // A comment which belongs to b\n  \"b\": \"some value\",\n  \"numeric\": [\n    1,\n    2,\n    3\n  ]\n}\n/* Comment below the main JSON object\n...\n...\n*/",
    );
}

#[test]
fn sorting_a_complicated_jsonc_object_2() {
    test_sort(
        "/*\n\nadding some comment before the actual JSON file\n\n*/ {\n    \"webviewContentExternalBaseUrlTemplate\": \"https://{{uuid}}.vscode-cdn.net/insider/ef65ac1ba57f57f2a3961bfe94aa20481caca4c6/out/vs/workbench/contrib/webview/browser/pre/\",\n    // some other comment\n    \"builtInExtensions\": [\n        {\n            \"name\": \"ms-vscode.js-debug-companion\", /** adding some more comments **/\n            \"version\": \"1.0.18\",\n            \"repo\": \"https://github.com/microsoft/vscode-js-debug-companion\",\n            \"metadata\": {\n                \"id\": \"99cb0b7f-7354-4278-b8da-6cc79972169d\",\n                \"publisherId\": {\n                    \"publisherId\": \"5f5636e7-69ed-4afe-b5d6-8d231fb3d3ee\",\n                    \"publisherName\": \"ms-vscode\" // comment\n                    ,\n                    \"displayName\": \"Microsoft\",\n                    \"flags\": \"verified\"\n                },\n                \"publisherDisplayName\": \"Microsoft\"\n            }\n        },\n        {\n            \"name\": \"ms-vscode.js-debug\", /** adding some more comments\n            ...\n            ...\n            */ \"version\": \"1.75.1\",\n            \"repo\": \"https://github.com/microsoft/vscode-js-debug\",\n            \"metadata\": {\n                \"id\": \"25629058-ddac-4e17-abba-74678e126c5d\",\n                \"publisherId\": {\n                    \"publisherId\": \"5f5636e7-69ed-4afe-b5d6-8d231fb3d3ee\",\n                    \"publisherName\": \"ms-vscode\",\n                    \"displayName\": \"Microsoft\",\n                    \"flags\": \"verified\"\n                },\n                \"publisherDisplayName\": \"Microsoft\"\n            }\n           // some more comments at the end after all properties\n        },\n        {\n            \"name\": \"ms-vscode.vscode-js-profile-table\",\n            \"version\": \"1.0.3\",\n            \"repo\": \"https://github.com/microsoft/vscode-js-profile-visualizer\",\n            \"metadata\": {\n                \"id\": \"7e52b41b-71ad-457b-ab7e-0620f1fc4feb\",\n                \"publisherId\": {\n                    \"publisherId\": \"5f5636e7-69ed-4afe-b5d6-8d231fb3d3ee\",\n                    \"publisherName\": \"ms-vscode\",\n                    \"displayName\": \"Microsoft\",\n                    \"flags\": \"verified\"\n                },\n                \"publisherDisplayName\": \"Microsoft\"\n            }\n        } \n    ] // comment on the end of an array\n}",
        "/*\n\nadding some comment before the actual JSON file\n\n*/ {\n  // some other comment\n  \"builtInExtensions\": [\n    {\n      \"metadata\": {\n        \"id\": \"99cb0b7f-7354-4278-b8da-6cc79972169d\",\n        \"publisherDisplayName\": \"Microsoft\",\n        \"publisherId\": {\n          \"displayName\": \"Microsoft\",\n          \"flags\": \"verified\",\n          \"publisherId\": \"5f5636e7-69ed-4afe-b5d6-8d231fb3d3ee\",\n          \"publisherName\": \"ms-vscode\" // comment\n        }\n      },\n      \"name\": \"ms-vscode.js-debug-companion\", /** adding some more comments **/\n      \"repo\": \"https://github.com/microsoft/vscode-js-debug-companion\",\n      \"version\": \"1.0.18\"\n    },\n    {\n      \"metadata\": {\n        \"id\": \"25629058-ddac-4e17-abba-74678e126c5d\",\n        \"publisherDisplayName\": \"Microsoft\",\n        \"publisherId\": {\n          \"displayName\": \"Microsoft\",\n          \"flags\": \"verified\",\n          \"publisherId\": \"5f5636e7-69ed-4afe-b5d6-8d231fb3d3ee\",\n          \"publisherName\": \"ms-vscode\"\n        }\n      },\n      // some more comments at the end after all properties\n      \"name\": \"ms-vscode.js-debug\", /** adding some more comments\n            ...\n            ...\n            */\n      \"repo\": \"https://github.com/microsoft/vscode-js-debug\",\n      \"version\": \"1.75.1\"\n    },\n    {\n      \"metadata\": {\n        \"id\": \"7e52b41b-71ad-457b-ab7e-0620f1fc4feb\",\n        \"publisherDisplayName\": \"Microsoft\",\n        \"publisherId\": {\n          \"displayName\": \"Microsoft\",\n          \"flags\": \"verified\",\n          \"publisherId\": \"5f5636e7-69ed-4afe-b5d6-8d231fb3d3ee\",\n          \"publisherName\": \"ms-vscode\"\n        }\n      },\n      \"name\": \"ms-vscode.vscode-js-profile-table\",\n      \"repo\": \"https://github.com/microsoft/vscode-js-profile-visualizer\",\n      \"version\": \"1.0.3\"\n    }\n  ], // comment on the end of an array\n  \"webviewContentExternalBaseUrlTemplate\": \"https://{{uuid}}.vscode-cdn.net/insider/ef65ac1ba57f57f2a3961bfe94aa20481caca4c6/out/vs/workbench/contrib/webview/browser/pre/\"\n}",
    );
}

#[test]
fn sorting_a_deeply_nested_jsonc_object() {
    test_sort(
        "{\n    \"a\" : {\n        \"y\" : {},\n        \"a\" : {}\n    }, // comment\n\n   \"b\" : [\n        [\n            { /* some comment\n            */\n                \"b\" : {\n                    \"y\" : [],\n                    \"x\" : []\n                },\n                \"a\" : {\n                    \"z\" : [],\n                    \"m\" : [],\n                    \"b\" : {\n                        \"b\" : 1,\n                        \"a\" : 2\n                    }\n                },\n                \"c\" : {\n                    \"a\" : {\n                        \"b\" : 1\n                    }\n                   // comment at the end\n                }\n            }\n        ]\n    ]\n}",
        "{\n  \"a\": {\n    \"a\": {},\n    \"y\": {}\n  }, // comment\n  \"b\": [\n    [\n      { /* some comment\n            */\n        \"a\": {\n          \"b\": {\n            \"a\": 2,\n            \"b\": 1\n          },\n          \"m\": [],\n          \"z\": []\n        },\n        \"b\": {\n          \"x\": [],\n          \"y\": []\n        },\n        \"c\": {\n          \"a\": {\n            \"b\": 1\n          }\n          // comment at the end\n        }\n      }\n    ]\n  ]\n}",
    );
}

#[test]
fn sorting_a_simple_jsonc_document_where_the_outer_container_is_an_array() {
    test_sort(
        "[\n    {\n        \"hi\": 1\n    },\n\n    // some comment\n\n    {\n        \"b\" : 2,\n        \"a\" : 1\n    }\n]",
        "[\n  {\n    \"hi\": 1\n  },\n  // some comment\n  {\n    \"a\": 1,\n    \"b\": 2\n  }\n]",
    );
}

#[test]
fn sorting_a_complicated_jsonc_object_3() {
    test_sort(
        "{\n    \"type\": \"array\",\n    \"items\": {\n        /* multi-line comment\n        ..\n        */\n        \"oneOf\": [ /* multi-line comment\n                ..\n                */\n            {\n                \"type\": \"object\",\n                \"required\": [ /* multi-line comment\n                            ..\n                            */\n                    \"name\",\n                    \"prependLicenseText\"\n                   /* multi-line comment\n                    ..\n                    */,\n                    {\n                        \"name\" : \"some property\", /* multi-line comment\n                        ..\n                        */\n                        \"value\" : \"some value\" // some value\n                        /* multi-line comment\n                        ..\n                        */\n                    }\n                ],\n                // one-line comment\n                \"properties\": { // one-line comment\n                    \"name\": {\n                        \"type\": \"string\",\n                        \"description\": \"The name of the dependency\" // one-line comment\n                    },\n                    \"fullLicenseText\": {\n                        \"type\": \"array\",\n                        \"description\": \"The complete license text of the dependency\",\n                        \"items\": {\n                            \"type\": \"string\"\n                            /* multi-line comment\n                            ..\n                            */\n                        },\n                        \"array\" : [\n                            {\n                                \"key\" : \"value\" /* multi-line comment\n                                ..\n                                */\n                            }\n                        ]\n                    },\n                    // one-line comment\n                    \"prependLicenseText\": {\n                        \"type\": \"array\", /* multi-line comment\n                        ..\n                        */\n                        \"description\": \"A piece of text to prepend to the auto-detected license text of the dependency\",\n                        /* multi-line comment\n                        ..\n                        */\n                        \"items\": {\n                            \"type\": \"string\"\n                        }\n                    }\n                }\n            },\n            {\n                \"type\": \"object\",\n                \"required\": [ // one-line comment\n                     \"name\",\n                    // one-line comment\n                    \"fullLicenseText\"\n                ],\n                \"properties\": {\n                    /* multi-line comment\n                    ..\n                    */\n                    \"name\": {\n                        \"type\": \"string\",\n                        \"description\": \"The name of the dependency\"\n                    },\n                    \"fullLicenseText\": {\n                        \"type\": \"array\",\n                        \"description\": \"The complete license text of the dependency\",\n                        \"items\": {\n                            \"type\": \"string\"\n                        }\n                    },\n                    /* multi-line comment\n                    ..\n                    */\n                    \"prependLicenseText\": {\n                        \"type\": \"array\",\n                        \"description\": \"A piece of text to prepend to the auto-detected license text of the dependency\", /* multi-line comment\n                        ..\n                        */\n                        \"items\": {\n                            \"type\": \"string\"\n                        }\n                    }\n                }\n            },\n            {\n                \"type\": \"object\",\n                \"required\": [\n                    \"name\",\n                    /* multi-line comment\n                    ..\n                    */\n                    \"fullLicenseTextUri\"\n                ],\n                \"properties\": {\n                    \"name\": {\n                        \"type\": \"string\", /* multi-line comment\n                        ..\n                        */\n                         \"description\": \"The name of the dependency\"\n                    },\n                    \"fullLicenseTextUri\": {\n                        \"type\": \"string\",\n                        \"description\": \"The URI to the license text of this repository\",\n                        \"format\": \"uri\" /* multi-line comment\n                        ..\n                        */\n                    },\n                    // one-line comment\n                    \"prependLicenseText\": { // one-line comment\n                        \"type\": \"array\",\n                        \"description\": \"A piece of text to prepend to the auto-detected license text of the dependency\",\n                        \"items\": {\n                            \"type\": \"string\"\n                        }\n                    } /* multi-line comment\n                    ..\n                    */\n                }\n            } /* multi-line comment\n            ..\n            */\n        ]\n    } // one-line comment\n}",
        "{\n  \"items\": {\n    /* multi-line comment\n        ..\n        */\n    \"oneOf\": [ /* multi-line comment\n                ..\n                */\n      {\n        // one-line comment\n        \"properties\": { // one-line comment\n          \"fullLicenseText\": {\n            \"array\": [\n              {\n                \"key\": \"value\" /* multi-line comment\n                                ..\n                                */\n              }\n            ],\n            \"description\": \"The complete license text of the dependency\",\n            \"items\": {\n              \"type\": \"string\"\n              /* multi-line comment\n                            ..\n                            */\n            },\n            \"type\": \"array\"\n          },\n          \"name\": {\n            \"description\": \"The name of the dependency\", // one-line comment\n            \"type\": \"string\"\n          },\n          // one-line comment\n          \"prependLicenseText\": {\n            \"description\": \"A piece of text to prepend to the auto-detected license text of the dependency\",\n            /* multi-line comment\n                        ..\n                        */\n            \"items\": {\n              \"type\": \"string\"\n            },\n            \"type\": \"array\" /* multi-line comment\n                        ..\n                        */\n          }\n        },\n        \"required\": [ /* multi-line comment\n                            ..\n                            */\n          \"name\",\n          \"prependLicenseText\"\n          /* multi-line comment\n                    ..\n                    */ ,\n          {\n            \"name\": \"some property\", /* multi-line comment\n                        ..\n                        */\n            \"value\": \"some value\" // some value\n            /* multi-line comment\n                        ..\n                        */\n          }\n        ],\n        \"type\": \"object\"\n      },\n      {\n        \"properties\": {\n          \"fullLicenseText\": {\n            \"description\": \"The complete license text of the dependency\",\n            \"items\": {\n              \"type\": \"string\"\n            },\n            \"type\": \"array\"\n          },\n          /* multi-line comment\n                    ..\n                    */\n          \"name\": {\n            \"description\": \"The name of the dependency\",\n            \"type\": \"string\"\n          },\n          /* multi-line comment\n                    ..\n                    */\n          \"prependLicenseText\": {\n            \"description\": \"A piece of text to prepend to the auto-detected license text of the dependency\", /* multi-line comment\n                        ..\n                        */\n            \"items\": {\n              \"type\": \"string\"\n            },\n            \"type\": \"array\"\n          }\n        },\n        \"required\": [ // one-line comment\n          \"name\",\n          // one-line comment\n          \"fullLicenseText\"\n        ],\n        \"type\": \"object\"\n      },\n      {\n        \"properties\": {\n          \"fullLicenseTextUri\": {\n            \"description\": \"The URI to the license text of this repository\",\n            \"format\": \"uri\", /* multi-line comment\n                        ..\n                        */\n            \"type\": \"string\"\n          },\n          \"name\": {\n            \"description\": \"The name of the dependency\",\n            \"type\": \"string\" /* multi-line comment\n                        ..\n                        */\n          },\n          // one-line comment\n          \"prependLicenseText\": { // one-line comment\n            \"description\": \"A piece of text to prepend to the auto-detected license text of the dependency\",\n            \"items\": {\n              \"type\": \"string\"\n            },\n            \"type\": \"array\"\n          } /* multi-line comment\n                    ..\n                    */\n        },\n        \"required\": [\n          \"name\",\n          /* multi-line comment\n                    ..\n                    */\n          \"fullLicenseTextUri\"\n        ],\n        \"type\": \"object\"\n      } /* multi-line comment\n            ..\n            */\n    ]\n  }, // one-line comment\n  \"type\": \"array\"\n}",
    );
}

#[test]
fn sorting_a_complicated_jsonc_object_4() {
    test_sort(
        "/** multi-line or block comment\n        ...\n        ...\n        */\n// one line comment\n{\n    \"version\": \"2.0.0\",\n    \"tasks\": [ // one line comment\n        { /** multi-line or block comment\n        ...\n        ...\n        */\n            \"type\": \"npm\" /** multi-line or block comment\n            ...\n            ...\n            */,\n            \"script\": \"watch-clientd\",\n            \"label\": \"Core - Build\", /** multi-line or block comment\n            ...\n            ...\n            */\n            \"isBackground\": true,\n            // one line comment\n            \"presentation\": {\n                \"reveal\": \"never\",\n                \"group\": \"buildWatchers\", // one line comment\n                \"close\": false\n            },\n            \"problemMatcher\": {\n                \"owner\": \"typescript\",\n                \"applyTo\": \"closedDocuments\",\n                \"fileLocation\": [\n                    /** multi-line or block comment\n                    ...\n                    ...\n                    */\n                    \"absolute\"\n                ],\n                \"pattern\": {\n                    \"regexp\": \"Error: ([^(]+)\\((\\d+|\\d+,\\d+|\\d+,\\d+,\\d+,\\d+)\\): (.*)$\",\n                    \"file\": 1,\n                    \"location\": 2, /** multi-line or block comment\n                    ...\n                    ...\n                    */\n                    \"message\": 3\n                },\n                // one line comment\n                \"background\": {\n                    \"beginsPattern\": \"Starting compilation...\",\n                    \"endsPattern\": \"Finished compilation with\"\n                }\n            } // one line comment\n        },\n        { /** multi-line or block comment\n        ...\n        ...\n        */\n            \"type\": \"npm\",\n            \"script\": \"watch-extensionsd\",\n            \"label\": \"Ext - Build\",\n            \"isBackground\": true,\n            \"presentation\": {\n                \"reveal\": \"never\",\n                \"group\": \"buildWatchers\",\n                \"close\": false\n            },\n            \"problemMatcher\": {\n                \"owner\": \"typescript\",\n                \"applyTo\": \"closedDocuments\",\n                \"fileLocation\": [ // one line comment\n                    \"absolute\"\n                ],\n                \"pattern\": { /** multi-line or block comment\n                ...\n                ...\n                */\n                    \"regexp\": \"Error: ([^(]+)\\((\\d+|\\d+,\\d+|\\d+,\\d+,\\d+,\\d+)\\): (.*)$\",\n                    \"file\": 1,\n                    \"location\": 2,\n                    \"message\": 3\n                },\n                \"background\": {\n                    \"beginsPattern\": \"Starting compilation\",\n                    \"endsPattern\": \"Finished compilation\"\n                }\n            }\n        }\n    ]\n}\n\n/** multi-line or block comment\n...\n...\n*/",
        "/** multi-line or block comment\n        ...\n        ...\n        */\n// one line comment\n{\n  \"tasks\": [ // one line comment\n    { /** multi-line or block comment\n        ...\n        ...\n        */\n      \"isBackground\": true,\n      \"label\": \"Core - Build\", /** multi-line or block comment\n            ...\n            ...\n            */\n      // one line comment\n      \"presentation\": {\n        \"close\": false,\n        \"group\": \"buildWatchers\", // one line comment\n        \"reveal\": \"never\"\n      },\n      \"problemMatcher\": {\n        \"applyTo\": \"closedDocuments\",\n        // one line comment\n        \"background\": {\n          \"beginsPattern\": \"Starting compilation...\",\n          \"endsPattern\": \"Finished compilation with\"\n        },\n        \"fileLocation\": [\n          /** multi-line or block comment\n                    ...\n                    ...\n                    */\n          \"absolute\"\n        ],\n        \"owner\": \"typescript\",\n        \"pattern\": {\n          \"file\": 1,\n          \"location\": 2, /** multi-line or block comment\n                    ...\n                    ...\n                    */\n          \"message\": 3,\n          \"regexp\": \"Error: ([^(]+)\\((\\d+|\\d+,\\d+|\\d+,\\d+,\\d+,\\d+)\\): (.*)$\"\n        }\n      }, // one line comment\n      \"script\": \"watch-clientd\",\n      \"type\": \"npm\" /** multi-line or block comment\n            ...\n            ...\n            */\n    },\n    { /** multi-line or block comment\n        ...\n        ...\n        */\n      \"isBackground\": true,\n      \"label\": \"Ext - Build\",\n      \"presentation\": {\n        \"close\": false,\n        \"group\": \"buildWatchers\",\n        \"reveal\": \"never\"\n      },\n      \"problemMatcher\": {\n        \"applyTo\": \"closedDocuments\",\n        \"background\": {\n          \"beginsPattern\": \"Starting compilation\",\n          \"endsPattern\": \"Finished compilation\"\n        },\n        \"fileLocation\": [ // one line comment\n          \"absolute\"\n        ],\n        \"owner\": \"typescript\",\n        \"pattern\": { /** multi-line or block comment\n                ...\n                ...\n                */\n          \"file\": 1,\n          \"location\": 2,\n          \"message\": 3,\n          \"regexp\": \"Error: ([^(]+)\\((\\d+|\\d+,\\d+|\\d+,\\d+,\\d+,\\d+)\\): (.*)$\"\n        }\n      },\n      \"script\": \"watch-extensionsd\",\n      \"type\": \"npm\"\n    }\n  ],\n  \"version\": \"2.0.0\"\n}\n/** multi-line or block comment\n...\n...\n*/",
    );
}

#[test]
fn sorting_a_complicated_jsonc_object_5() {
    test_sort(
        "{ /** multi-line or block comment\n...\n...\n*/\n    \"registrations\": [ /** multi-line or block comment\n    ...\n    ...\n    */\n        { /** multi-line or block comment\n        ...\n        ...\n        */\n            \"component\": { /** multi-line or block comment\n            ...\n            ...\n            */\n                \"type\": \"git\",\n                \"git\": {\n                    \"name\": \"textmate/markdown.tmbundle\" /** multi-line or block comment\n                    ...\n                    ...\n                    */,\n                    \"repositoryUrl\": /** multi-line or block comment\n                    ...\n                    ...\n                    */ \"https://github.com/textmate/markdown.tmbundle\",\n                    \"commitHash\": /** multi-line or block comment\n                    ...\n                    ...\n                    */ \"11cf764606cb2cde54badb5d0e5a0758a8871c4b\"\n                }\n            },\n            \"licenseDetail\": /** multi-line or block comment\n            ...\n            ...\n            */ [ /** multi-line or block comment\n            ...\n            ...\n            */\n                \"Copyright (c) markdown.tmbundle authors\",\n                \"\" /** multi-line or block comment\n                ...\n                ...\n                */,\n                \"If not otherwise specified (see below), files in this repository fall under the following license:\",\n                \"\" /** multi-line or block comment\n                ...\n                ...\n                */,\n                \"Permission to copy, use, modify, sell and distribute this\", /** multi-line or block comment\n                ...\n                ...\n                */\n                \"\",\n                \"An exception is made for files in readable text which contain their own license information,\",\n                \"or files where an accompanying file exists (in the same directory) with a \"-license\" suffix added\",\n                \"to the base-name name of the original file, and an extension of txt, html, or similar. For example\",\n                \"\"tidy\" is accompanied by \"tidy-license.txt\".\"\n            ] /** multi-line or block comment\n            ...\n            ...\n            */,\n            \"license\": \"TextMate Bundle License\",\n            \"version\": \"0.0.0\"\n        } /** multi-line or block comment\n        ...\n        ...\n        */,\n\n        /** multi-line or block comment\n    ...\n    ...\n    */\n        {\n            \"component\": {\n                \"type\": \"git\",\n                \"git\": {\n                    \"name\": \"microsoft/vscode-markdown-tm-grammar\",\n                    \"repositoryUrl\": \"https://github.com/microsoft/vscode-markdown-tm-grammar\",\n                    \"commitHash\": \"443261e8f75b2eaa8b36a2613fe7c4354208260a\"\n                }\n            },\n            /** multi-line or block comment\n    ...\n    ...\n    */\n            \"license\": \"MIT\",\n            \"version\": \"1.0.0\"\n        }\n    ],\n    \"version\": 1\n} /** multi-line or block comment\n...\n...\n*/",
        "{ /** multi-line or block comment\n...\n...\n*/\n  \"registrations\": [ /** multi-line or block comment\n    ...\n    ...\n    */\n    { /** multi-line or block comment\n        ...\n        ...\n        */\n      \"component\": { /** multi-line or block comment\n            ...\n            ...\n            */\n        \"git\": {\n          \"commitHash\": /** multi-line or block comment\n                    ...\n                    ...\n                    */ \"11cf764606cb2cde54badb5d0e5a0758a8871c4b\",\n          \"name\": \"textmate/markdown.tmbundle\" /** multi-line or block comment\n                    ...\n                    ...\n                    */,\n          \"repositoryUrl\": /** multi-line or block comment\n                    ...\n                    ...\n                    */ \"https://github.com/textmate/markdown.tmbundle\"\n        },\n        \"type\": \"git\"\n      },\n      \"license\": \"TextMate Bundle License\",\n      \"licenseDetail\": /** multi-line or block comment\n            ...\n            ...\n            */ [ /** multi-line or block comment\n            ...\n            ...\n            */\n        \"Copyright (c) markdown.tmbundle authors\",\n        \"\" /** multi-line or block comment\n                ...\n                ...\n                */,\n        \"If not otherwise specified (see below), files in this repository fall under the following license:\",\n        \"\" /** multi-line or block comment\n                ...\n                ...\n                */,\n        \"Permission to copy, use, modify, sell and distribute this\", /** multi-line or block comment\n                ...\n                ...\n                */\n        \"\",\n        \"An exception is made for files in readable text which contain their own license information,\",\n        \"or files where an accompanying file exists (in the same directory) with a \"-license\" suffix added\",\n        \"to the base-name name of the original file, and an extension of txt, html, or similar. For example\",\n        \"\"tidy\" is accompanied by \"tidy-license.txt\".\"\n      ] /** multi-line or block comment\n            ...\n            ...\n            */,\n      \"version\": \"0.0.0\"\n    } /** multi-line or block comment\n        ...\n        ...\n        */,\n    /** multi-line or block comment\n    ...\n    ...\n    */\n    {\n      \"component\": {\n        \"git\": {\n          \"commitHash\": \"443261e8f75b2eaa8b36a2613fe7c4354208260a\",\n          \"name\": \"microsoft/vscode-markdown-tm-grammar\",\n          \"repositoryUrl\": \"https://github.com/microsoft/vscode-markdown-tm-grammar\"\n        },\n        \"type\": \"git\"\n      },\n      /** multi-line or block comment\n    ...\n    ...\n    */\n      \"license\": \"MIT\",\n      \"version\": \"1.0.0\"\n    }\n  ],\n  \"version\": 1\n} /** multi-line or block comment\n...\n...\n*/",
    );
}

#[test]
fn sorting_a_complicated_jsonc_object_6() {
    test_sort(
        "{\n    \"tsImportSorter.configuration.groupRules\": [\n        [{ \"builtin\": true }, \"^[@][^/]\", {}],\n        [\n            \"^@(/|$)\",\n            \"^apis?(/|$)\",\n            \"^assets(/|$)\",\n            \"^components?(/|$)\",\n            \"^pages?(/|$)\",\n            \"^slices?(/|$)\",\n            \"^store(/|$)\",\n            \"^typings?(/|$)\",\n            \"^utils(/|$)\"\n        ],\n        { \"flags\": \"named\", \"regex\": \"^[.]\" },\n        [{ \"flags\": \"scripts\" }, { \"flags\": \"scripts\", \"regex\": \"[.]((css)|(less)|(scss))$\" }]\n    ]\n}",
        "{\n  \"tsImportSorter.configuration.groupRules\": [\n    [\n      {\n        \"builtin\": true\n      },\n      \"^[@][^/]\",\n      {}\n    ],\n    [\n      \"^@(/|$)\",\n      \"^apis?(/|$)\",\n      \"^assets(/|$)\",\n      \"^components?(/|$)\",\n      \"^pages?(/|$)\",\n      \"^slices?(/|$)\",\n      \"^store(/|$)\",\n      \"^typings?(/|$)\",\n      \"^utils(/|$)\"\n    ],\n    {\n      \"flags\": \"named\",\n      \"regex\": \"^[.]\"\n    },\n    [\n      {\n        \"flags\": \"scripts\"\n      },\n      {\n        \"flags\": \"scripts\",\n        \"regex\": \"[.]((css)|(less)|(scss))$\"\n      }\n    ]\n  ]\n}",
    );
}

#[test]
fn sorting_a_json_object_with_mixed_case_keys() {
    test_sort(
        "{\n  \"tEst\": \"tEst\",\n  \"tesT\": \"tesT\",\n  \"teSt\": \"teSt\",\n  \"Test\": \"Test\",\n  \"test\": \"test\"\n}",
        "{\n  \"test\": \"test\",\n  \"tesT\": \"tesT\",\n  \"teSt\": \"teSt\",\n  \"tEst\": \"tEst\",\n  \"Test\": \"Test\"\n}",
    );
}

#[test]
fn sorting_an_already_sorted_json_object_with_mixed_case_keys() {
    test_sort(
        "{\n  \"test\": \"test\",\n  \"tesT\": \"tesT\",\n  \"teSt\": \"teSt\",\n  \"tEst\": \"tEst\",\n  \"Test\": \"Test\"\n}",
        "{\n  \"test\": \"test\",\n  \"tesT\": \"tesT\",\n  \"teSt\": \"teSt\",\n  \"tEst\": \"tEst\",\n  \"Test\": \"Test\"\n}",
    );
}

#[test]
fn sorting_symbols_before_letters() {
    test_sort(
        "{\n  \"Test\": \"Test\",\n  \"test\": \"test\",\n  \"[test]: \"test\n}",
        "{\n  \"[test]: \"test,\n  \"test\": \"test\",\n  \"Test\": \"Test\"\n}",
    );
}
