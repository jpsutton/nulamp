//! Spec 12 §2.3: title formatting acceptance table.

use std::collections::HashMap;

use nulamp_tf::{DEFAULT_FORMAT, format};

fn fields() -> HashMap<String, String> {
    [
        ("artist", "Daft Punk"),
        ("title", "One More Time"),
        ("track", "3/14"),
        ("filename", "/music/Daft Punk - One More Time.mp3"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

fn check(fmt: &str, expected: &str) {
    assert_eq!(format(fmt, &fields()), expected, "format: {fmt}");
}

#[test]
fn default_format() {
    assert_eq!(
        DEFAULT_FORMAT,
        "[%artist% - ]$if2(%title%,$filepart(%filename%))"
    );
    check(DEFAULT_FORMAT, "Daft Punk - One More Time");
}

#[test]
fn missing_field_hides_section() {
    check("[%album% - ]%title%", "One More Time");
}

#[test]
fn if_missing_field() {
    check("$if(%album%,yes,no)", "no");
}

#[test]
fn if_literal_is_false() {
    check("$if(abc,yes,no)", "no");
}

#[test]
fn section_without_fields() {
    check("[abc]", "");
}

#[test]
fn section_true_through_null() {
    check("[x$null(%artist%)]", "x");
}

#[test]
fn whitespace_not_trimmed() {
    check("$if(%artist%, a, b)", " a");
}

#[test]
fn bare_parentheses_are_an_error() {
    check("%artist% (live)", "[SYNTAX ERROR IN FORMATTING STRING]");
}

#[test]
fn quoted_parentheses() {
    check("%artist% '(live)'", "Daft Punk (live)");
}

#[test]
fn num_of_track() {
    check("$num(%track%,2)", "03");
}

#[test]
fn upper_left() {
    check("$upper($left(%artist%,4))", "DAFT");
}

#[test]
fn caps_after_punctuation() {
    check("$caps(don't stop)", "Don'T Stop");
}

#[test]
fn abbr_roman() {
    check("$abbr(Final Fantasy VI Advance)", "FFVIA");
}

#[test]
fn unknown_function() {
    check("$foo(x)", "[UNKNOWN FUNCTION]");
}

#[test]
fn if2_wrong_count() {
    check("$if2(%title%)", "[INVALID $IF SYNTAX]");
}

#[test]
fn div_by_zero_skipped() {
    check("$div(7,0)", "7");
}

#[test]
fn muldiv_rounds() {
    check("$muldiv(7,1,2)", "4");
}

#[test]
fn replace_field() {
    check("$replace(%title%,One,Two)", "Two More Time");
}

#[test]
fn directory_of_filename() {
    check("$directory(%filename%)", "music");
}

#[test]
fn put_get() {
    check("$put(a,x)$get(a)", "xx");
}

#[test]
fn escapes() {
    check("%%$$''", "%$'");
}

#[test]
fn pathsafe() {
    check("$pathsafe(a/b:c?)", "a-b-c_");
}

#[test]
fn select() {
    check("$select(2,a,b,c)", "b");
}

#[test]
fn decode_case_insensitive() {
    check("$decode(%artist%,daft punk,DP,other)", "DP");
}
