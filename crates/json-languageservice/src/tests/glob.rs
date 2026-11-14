use crate::glob::create_regex;

fn assert_match(pattern: &str, input: &str, expected: bool) {
    let regex = create_regex(pattern, true, true).expect("valid glob");
    assert_eq!(
        regex.find(input).is_some(),
        expected,
        "pattern: {pattern}, input: {input}"
    );
}

#[test]
fn glob_matching_where_filematch_is_a_wildcard_pattern_contains_no_double_star_and_denotes_filename_only()
 {
    assert_match("**/*.foo.json", "file:///folder/a.foo.json", true);
    assert_match("**/*.foo.json", "file:///folder/a.bar.json", false);
}

#[test]
fn glob_matching_where_filematch_is_a_wildcard_pattern_contains_no_double_star_and_denotes_a_path()
{
    assert_match("**/foo/*/bar.json", "file:///folder/foo/bat/bar.json", true);
    assert_match("**/foo/*/bar.json", "file:///folder/a.bar.json", false);
    assert_match("**/foo/*/bar.json", "file:///folder/foo/bar.json", false);
    assert_match(
        "**/foo/*/bar.json",
        "file:///folder/foo/can/be/as/deep/as/the/ocean/floor/bar.json",
        false,
    );
    assert_match(
        "**/foo/*/bar.json",
        "file:///folder/foo/bar.json/bat/bar.json",
        false,
    );
    assert_match("**/foo/*/bar.json", "file:///folder/foo.bar.json", false);
    assert_match(
        "**/foo/*/bar.json",
        "file:///folder/foo.bat/bar.json",
        false,
    );
    assert_match(
        "**/foo/*/bar.json",
        "file:///folder/foo/bar.json/bat.json",
        false,
    );
    assert_match("**/foo/*/bar.json", "file:///folder/.foo/bar.json", false);
    assert_match(
        "**/foo/*/bar.json",
        "file:///folder/.foo/bat/bar.json",
        false,
    );
    assert_match(
        "**/foo/*/bar.json",
        "file:///folder/.foo/bat/man/bar.json",
        false,
    );
}

#[test]
fn glob_matching_where_filematch_is_a_wildcard_pattern_contains_double_star_and_denotes_a_path() {
    assert_match("**/foo/**/bar.json", "file:///folder/foo/bar.json", true);
    assert_match(
        "**/foo/**/bar.json",
        "file:///folder/foo/bat/bar.json",
        true,
    );
    assert_match(
        "**/foo/**/bar.json",
        "file:///folder/foo/can/be/as/deep/as/the/ocean/floor/bar.json",
        true,
    );
    assert_match(
        "**/foo/**/bar.json",
        "file:///folder/foo/bar.json/bat/bar.json",
        true,
    );
    assert_match("**/foo/**/bar.json", "file:///folder/a.bar.json", false);
    assert_match("**/foo/**/bar.json", "file:///folder/foo.bar.json", false);
    assert_match(
        "**/foo/**/bar.json",
        "file:///folder/foo.bat/bar.json",
        false,
    );
    assert_match(
        "**/foo/**/bar.json",
        "file:///folder/foo/bar.json/bat.json",
        false,
    );
    assert_match("**/foo/**/bar.json", "file:///folder/.foo/bar.json", false);
    assert_match(
        "**/foo/**/bar.json",
        "file:///folder/.foo/bat/bar.json",
        false,
    );
    assert_match(
        "**/foo/**/bar.json",
        "file:///folder/.foo/bat/man/bar.json",
        false,
    );
    assert_match(
        "**/foo/**/bar.json",
        "file:///folder/foo?foo/bat/man/bar.json",
        false,
    );
}

#[test]
fn simple() {
    assert_match("node_modules", "node_modules", true);
    assert_match("node_modules", "node_module", false);
    assert_match("node_modules", "/node_modules", false);
    assert_match("node_modules", "test/node_modules", false);
    assert_match("test.txt", "test.txt", true);
    assert_match("test.txt", "test?txt", false);
    assert_match("test.txt", "/text.txt", false);
    assert_match("test.txt", "test/test.txt", false);
    assert_match("test(.txt", "test(.txt", true);
    assert_match("test(.txt", "test?txt", false);
    assert_match("qunit", "qunit", true);
    assert_match("qunit", "qunit.css", false);
    assert_match("qunit", "test/qunit", false);
    assert_match("/DNXConsoleApp/**/*.cs", "/DNXConsoleApp/Program.cs", true);
    assert_match(
        "/DNXConsoleApp/**/*.cs",
        "/DNXConsoleApp/foo/Program.cs",
        true,
    );
    assert_match("*", "", true);
}

#[test]
fn dot_hidden() {
    assert_match(".*", ".git", true);
    assert_match(".*", ".hidden.txt", true);
    assert_match(".*", "git", false);
    assert_match(".*", "hidden.txt", false);
    assert_match(".*", "path/.git", false);
    assert_match(".*", "path/.hidden.txt", false);
    assert_match("**/.*", ".git", true);
    assert_match("**/.*", ".hidden.txt", true);
    assert_match("**/.*", "git", false);
    assert_match("**/.*", "hidden.txt", false);
    assert_match("**/.*", "path/.git", true);
    assert_match("**/.*", "path/.hidden.txt", true);
    assert_match("**/.*", "path/git", false);
    assert_match("**/.*", "pat.h/hidden.txt", false);
    assert_match("._*", "._git", true);
    assert_match("._*", "._hidden.txt", true);
    assert_match("._*", "git", false);
    assert_match("._*", "hidden.txt", false);
    assert_match("._*", "path/._git", false);
    assert_match("._*", "path/._hidden.txt", false);
    assert_match("**/._*", "._git", true);
    assert_match("**/._*", "._hidden.txt", true);
    assert_match("**/._*", "git", false);
    assert_match("**/._*", "hidden._txt", false);
    assert_match("**/._*", "path/._git", true);
    assert_match("**/._*", "path/._hidden.txt", true);
    assert_match("**/._*", "path/git", false);
    assert_match("**/._*", "pat.h/hidden._txt", false);
}

#[test]
fn file_pattern() {
    assert_match("*.js", "foo.js", true);
    assert_match("*.js", "folder/foo.js", false);
    assert_match("*.js", "/node_modules/foo.js", false);
    assert_match("*.js", "foo.jss", false);
    assert_match("*.js", "some.js/test", false);
    assert_match("html.*", "html.js", true);
    assert_match("html.*", "html.txt", true);
    assert_match("html.*", "htm.txt", false);
    assert_match("*.*", "html.js", true);
    assert_match("*.*", "html.txt", true);
    assert_match("*.*", "htm.txt", true);
    assert_match("*.*", "folder/foo.js", false);
    assert_match("*.*", "/node_modules/foo.js", false);
    assert_match("node_modules/test/*.js", "node_modules/test/foo.js", true);
    assert_match("node_modules/test/*.js", "folder/foo.js", false);
    assert_match("node_modules/test/*.js", "/node_module/test/foo.js", false);
    assert_match("node_modules/test/*.js", "foo.jss", false);
    assert_match("node_modules/test/*.js", "some.js/test", false);
}

#[test]
fn star() {
    assert_match("node*modules", "node_modules", true);
    assert_match("node*modules", "node_super_modules", true);
    assert_match("node*modules", "node_module", false);
    assert_match("node*modules", "/node_modules", false);
    assert_match("node*modules", "test/node_modules", false);
    assert_match("*", "html.js", true);
    assert_match("*", "html.txt", true);
    assert_match("*", "htm.txt", true);
    assert_match("*", "folder/foo.js", false);
    assert_match("*", "/node_modules/foo.js", false);
}

#[test]
fn file_folder_match() {
    assert_match("**/node_modules/**", "node_modules", true);
    assert_match("**/node_modules/**", "node_modules/", true);
    assert_match("**/node_modules/**", "a/node_modules", true);
    assert_match("**/node_modules/**", "a/node_modules/", true);
    assert_match("**/node_modules/**", "node_modules/foo", true);
    assert_match("**/node_modules/**", "foo/node_modules/foo/bar", true);
}

#[test]
fn questionmark() {
    assert_match("node?modules", "node_modules", true);
    assert_match("node?modules", "node_super_modules", false);
    assert_match("node?modules", "node_module", false);
    assert_match("node?modules", "/node_modules", false);
    assert_match("node?modules", "test/node_modules", false);
    assert_match("?", "h", true);
    assert_match("?", "html.txt", false);
    assert_match("?", "htm.txt", false);
    assert_match("?", "folder/foo.js", false);
    assert_match("?", "/node_modules/foo.js", false);
}

#[test]
fn globstar() {
    assert_match("**/*.js", "foo.js", true);
    assert_match("**/*.js", "folder/foo.js", true);
    assert_match("**/*.js", "/node_modules/foo.js", true);
    assert_match("**/*.js", "foo.jss", false);
    assert_match("**/*.js", "some.js/test", false);
    assert_match("**/*.js", "/some.js/test", false);
    assert_match("**/*.js", "/some.js/test", false);
    assert_match("**/project.json", "project.json", true);
    assert_match("**/project.json", "/project.json", true);
    assert_match("**/project.json", "some/folder/project.json", true);
    assert_match("**/project.json", "some/folder/file_project.json", false);
    assert_match("**/project.json", "some/folder/fileproject.json", false);
    assert_match("**/project.json", "some/rrproject.json", false);
    assert_match("test/**", "test", true);
    assert_match("test/**", "test/foo.js", true);
    assert_match("test/**", "test/other/foo.js", true);
    assert_match("test/**", "est/other/foo.js", false);
    assert_match("**", "foo.js", true);
    assert_match("**", "folder/foo.js", true);
    assert_match("**", "/node_modules/foo.js", true);
    assert_match("**", "foo.jss", true);
    assert_match("**", "some.js/test", true);
    assert_match("test/**/*.js", "test/foo.js", true);
    assert_match("test/**/*.js", "test/other/foo.js", true);
    assert_match("test/**/*.js", "test/other/more/foo.js", true);
    assert_match("test/**/*.js", "test/foo.ts", false);
    assert_match("test/**/*.js", "test/other/foo.ts", false);
    assert_match("test/**/*.js", "test/other/more/foo.ts", false);
    assert_match("**/**/*.js", "foo.js", true);
    assert_match("**/**/*.js", "folder/foo.js", true);
    assert_match("**/**/*.js", "/node_modules/foo.js", true);
    assert_match("**/**/*.js", "foo.jss", false);
    assert_match("**/**/*.js", "some.js/test", false);
    assert_match("**/node_modules/**/*.js", "foo.js", false);
    assert_match("**/node_modules/**/*.js", "folder/foo.js", false);
    assert_match("**/node_modules/**/*.js", "node_modules/foo.js", true);
    assert_match(
        "**/node_modules/**/*.js",
        "node_modules/some/folder/foo.js",
        true,
    );
    assert_match(
        "**/node_modules/**/*.js",
        "node_modules/some/folder/foo.ts",
        false,
    );
    assert_match("**/node_modules/**/*.js", "foo.jss", false);
    assert_match("**/node_modules/**/*.js", "some.js/test", false);
    assert_match(
        "{**/node_modules/**,**/.git/**,**/bower_components/**}",
        "node_modules",
        true,
    );
    assert_match(
        "{**/node_modules/**,**/.git/**,**/bower_components/**}",
        "/node_modules",
        true,
    );
    assert_match(
        "{**/node_modules/**,**/.git/**,**/bower_components/**}",
        "/node_modules/more",
        true,
    );
    assert_match(
        "{**/node_modules/**,**/.git/**,**/bower_components/**}",
        "some/test/node_modules",
        true,
    );
    assert_match(
        "{**/node_modules/**,**/.git/**,**/bower_components/**}",
        "some/test/node_modules",
        true,
    );
    assert_match(
        "{**/node_modules/**,**/.git/**,**/bower_components/**}",
        "C://some/test/node_modules",
        true,
    );
    assert_match(
        "{**/node_modules/**,**/.git/**,**/bower_components/**}",
        "C://some/test/node_modules/more",
        true,
    );
    assert_match(
        "{**/node_modules/**,**/.git/**,**/bower_components/**}",
        "bower_components",
        true,
    );
    assert_match(
        "{**/node_modules/**,**/.git/**,**/bower_components/**}",
        "bower_components/more",
        true,
    );
    assert_match(
        "{**/node_modules/**,**/.git/**,**/bower_components/**}",
        "/bower_components",
        true,
    );
    assert_match(
        "{**/node_modules/**,**/.git/**,**/bower_components/**}",
        "some/test/bower_components",
        true,
    );
    assert_match(
        "{**/node_modules/**,**/.git/**,**/bower_components/**}",
        "some/test/bower_components",
        true,
    );
    assert_match(
        "{**/node_modules/**,**/.git/**,**/bower_components/**}",
        "C://some/test/bower_components",
        true,
    );
    assert_match(
        "{**/node_modules/**,**/.git/**,**/bower_components/**}",
        "C://some/test/bower_components/more",
        true,
    );
    assert_match(
        "{**/node_modules/**,**/.git/**,**/bower_components/**}",
        ".git",
        true,
    );
    assert_match(
        "{**/node_modules/**,**/.git/**,**/bower_components/**}",
        "/.git",
        true,
    );
    assert_match(
        "{**/node_modules/**,**/.git/**,**/bower_components/**}",
        "some/test/.git",
        true,
    );
    assert_match(
        "{**/node_modules/**,**/.git/**,**/bower_components/**}",
        "some/test/.git",
        true,
    );
    assert_match(
        "{**/node_modules/**,**/.git/**,**/bower_components/**}",
        "C://some/test/.git",
        true,
    );
    assert_match(
        "{**/node_modules/**,**/.git/**,**/bower_components/**}",
        "tempting",
        false,
    );
    assert_match(
        "{**/node_modules/**,**/.git/**,**/bower_components/**}",
        "/tempting",
        false,
    );
    assert_match(
        "{**/node_modules/**,**/.git/**,**/bower_components/**}",
        "some/test/tempting",
        false,
    );
    assert_match(
        "{**/node_modules/**,**/.git/**,**/bower_components/**}",
        "some/test/tempting",
        false,
    );
    assert_match(
        "{**/node_modules/**,**/.git/**,**/bower_components/**}",
        "C://some/test/tempting",
        false,
    );
    assert_match("{**/package.json,**/project.json}", "package.json", true);
    assert_match("{**/package.json,**/project.json}", "/package.json", true);
    assert_match("{**/package.json,**/project.json}", "xpackage.json", false);
    assert_match("{**/package.json,**/project.json}", "/xpackage.json", false);
}

#[test]
fn issue_41724() {
    assert_match("some/**/*.js", "some/foo.js", true);
    assert_match("some/**/*.js", "some/folder/foo.js", true);
    assert_match("some/**/*.js", "something/foo.js", false);
    assert_match("some/**/*.js", "something/folder/foo.js", false);
    assert_match("some/**/*", "some/foo.js", true);
    assert_match("some/**/*", "some/folder/foo.js", true);
    assert_match("some/**/*", "something/foo.js", false);
    assert_match("some/**/*", "something/folder/foo.js", false);
}

#[test]
fn brace_expansion() {
    assert_match("*.{html,js}", "foo.js", true);
    assert_match("*.{html,js}", "foo.html", true);
    assert_match("*.{html,js}", "folder/foo.js", false);
    assert_match("*.{html,js}", "/node_modules/foo.js", false);
    assert_match("*.{html,js}", "foo.jss", false);
    assert_match("*.{html,js}", "some.js/test", false);
    assert_match("*.{html}", "foo.html", true);
    assert_match("*.{html}", "foo.js", false);
    assert_match("*.{html}", "folder/foo.js", false);
    assert_match("*.{html}", "/node_modules/foo.js", false);
    assert_match("*.{html}", "foo.jss", false);
    assert_match("*.{html}", "some.js/test", false);
    assert_match("{node_modules,testing}", "node_modules", true);
    assert_match("{node_modules,testing}", "testing", true);
    assert_match("{node_modules,testing}", "node_module", false);
    assert_match("{node_modules,testing}", "dtesting", false);
    assert_match("**/{foo,bar}", "foo", true);
    assert_match("**/{foo,bar}", "bar", true);
    assert_match("**/{foo,bar}", "test/foo", true);
    assert_match("**/{foo,bar}", "test/bar", true);
    assert_match("**/{foo,bar}", "other/more/foo", true);
    assert_match("**/{foo,bar}", "other/more/bar", true);
    assert_match("{foo,bar}/**", "foo", true);
    assert_match("{foo,bar}/**", "bar", true);
    assert_match("{foo,bar}/**", "foo/test", true);
    assert_match("{foo,bar}/**", "bar/test", true);
    assert_match("{foo,bar}/**", "foo/other/more", true);
    assert_match("{foo,bar}/**", "bar/other/more", true);
    assert_match("{**/*.d.ts,**/*.js}", "foo.js", true);
    assert_match("{**/*.d.ts,**/*.js}", "testing/foo.js", true);
    assert_match("{**/*.d.ts,**/*.js}", "testing/foo.js", true);
    assert_match("{**/*.d.ts,**/*.js}", "/testing/foo.js", true);
    assert_match("{**/*.d.ts,**/*.js}", "/testing/foo.js", true);
    assert_match("{**/*.d.ts,**/*.js}", "C:/testing/foo.js", true);
    assert_match("{**/*.d.ts,**/*.js}", "foo.d.ts", true);
    assert_match("{**/*.d.ts,**/*.js}", "testing/foo.d.ts", true);
    assert_match("{**/*.d.ts,**/*.js}", "testing/foo.d.ts", true);
    assert_match("{**/*.d.ts,**/*.js}", "/testing/foo.d.ts", true);
    assert_match("{**/*.d.ts,**/*.js}", "/testing/foo.d.ts", true);
    assert_match("{**/*.d.ts,**/*.js}", "C:/testing/foo.d.ts", true);
    assert_match("{**/*.d.ts,**/*.js}", "foo.d", false);
    assert_match("{**/*.d.ts,**/*.js}", "testing/foo.d", false);
    assert_match("{**/*.d.ts,**/*.js}", "testing/foo.d", false);
    assert_match("{**/*.d.ts,**/*.js}", "/testing/foo.d", false);
    assert_match("{**/*.d.ts,**/*.js}", "/testing/foo.d", false);
    assert_match("{**/*.d.ts,**/*.js}", "C:/testing/foo.d", false);
    assert_match("{**/*.d.ts,**/*.js,path/simple.jgs}", "foo.js", true);
    assert_match(
        "{**/*.d.ts,**/*.js,path/simple.jgs}",
        "testing/foo.js",
        true,
    );
    assert_match(
        "{**/*.d.ts,**/*.js,path/simple.jgs}",
        "testing/foo.js",
        true,
    );
    assert_match(
        "{**/*.d.ts,**/*.js,path/simple.jgs}",
        "/testing/foo.js",
        true,
    );
    assert_match(
        "{**/*.d.ts,**/*.js,path/simple.jgs}",
        "path/simple.jgs",
        true,
    );
    assert_match(
        "{**/*.d.ts,**/*.js,path/simple.jgs}",
        "/path/simple.jgs",
        false,
    );
    assert_match(
        "{**/*.d.ts,**/*.js,path/simple.jgs}",
        "/testing/foo.js",
        true,
    );
    assert_match(
        "{**/*.d.ts,**/*.js,path/simple.jgs}",
        "C:/testing/foo.js",
        true,
    );
    assert_match("{**/*.d.ts,**/*.js,foo.[0-9]}", "foo.5", true);
    assert_match("{**/*.d.ts,**/*.js,foo.[0-9]}", "foo.8", true);
    assert_match("{**/*.d.ts,**/*.js,foo.[0-9]}", "bar.5", false);
    assert_match("{**/*.d.ts,**/*.js,foo.[0-9]}", "foo.f", false);
    assert_match("{**/*.d.ts,**/*.js,foo.[0-9]}", "foo.js", true);
    assert_match("prefix/{**/*.d.ts,**/*.js,foo.[0-9]}", "prefix/foo.5", true);
    assert_match("prefix/{**/*.d.ts,**/*.js,foo.[0-9]}", "prefix/foo.8", true);
    assert_match(
        "prefix/{**/*.d.ts,**/*.js,foo.[0-9]}",
        "prefix/bar.5",
        false,
    );
    assert_match(
        "prefix/{**/*.d.ts,**/*.js,foo.[0-9]}",
        "prefix/foo.f",
        false,
    );
    assert_match(
        "prefix/{**/*.d.ts,**/*.js,foo.[0-9]}",
        "prefix/foo.js",
        true,
    );
}
