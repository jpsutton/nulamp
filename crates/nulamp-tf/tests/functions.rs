//! Behaviour of every function, field and syntax rule of spec 10 §5.3.

use std::collections::HashMap;

use nulamp_tf::{FieldSource, Formatter, MAX_LEN, SYNTAX_ERROR, SysTime, UNKNOWN_FUNCTION};

const NOW: SysTime = SysTime {
    year: 2026,
    month: 3,
    day: 7,
    hour: 9,
    minute: 5,
    second: 4,
};

fn fields() -> HashMap<String, String> {
    [
        ("artist", "Daft Punk"),
        ("title", "One More Time"),
        ("track", "3/14"),
        ("filename", "C:\\a\\b\\c.mp3"),
        ("multi", "line1\r\nline2\n"),
        ("wide", "ＤＡＦＴ"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

fn f(fmt: &str) -> String {
    Formatter::new(fmt).format_at(&fields(), NOW)
}

fn check(cases: &[(&str, &str)]) {
    for (fmt, expected) in cases {
        assert_eq!(f(fmt), *expected, "format: {fmt}");
    }
}

#[test]
fn syntax() {
    check(&[
        ("%ARTIST%", "Daft Punk"),
        ("%nosuch%x", "x"),
        ("a,b", "a,b"),
        ("'%artist%'", "%artist%"),
        ("'[a],(b)'", "[a],(b)"),
        ("it''s", "it's"),
        ("$UPPER(a)", "A"),
        ("$if(%artist%,'a,b',c)", "a,b"),
        ("[[%album%]x]", ""),
        ("[[%artist%]x]", "Daft Punkx"),
        ("[%album%[%artist%]]", "Daft Punk"),
        ("%multi%", "line1  line2 "),
        ("a$crlf()b", "a  b"),
    ]);
}

#[test]
fn syntax_errors() {
    for fmt in [
        "%artist", "'abc", "[abc", "$len(a", "a)", "a]", "(a)", "$len", "$", "x$(a)",
    ] {
        assert_eq!(f(fmt), SYNTAX_ERROR, "format: {fmt}");
    }
    let e = SYNTAX_ERROR;
    check(&[
        ("[%artist - ]%title%", &format!("{e}One More Time")),
        ("[a)b]c", &format!("{e}c")),
        ("$if2(%a,x)", "x"),
        ("$len([a)b)", "35"),
        ("$if(%x(y),a,b)", "b"),
        ("$left(a]b,1)", "["),
    ]);
}

#[test]
fn unknown_functions() {
    for name in [
        "and", "or", "not", "xor", "greater", "strcmp", "stricmp", "insert", "foo",
    ] {
        assert_eq!(f(&format!("${name}(1,1)")), UNKNOWN_FUNCTION);
    }
    check(&[
        ("$foo($puts(v,1))$get(v)", "[UNKNOWN FUNCTION]1"),
        ("[$foo(%artist%)]", UNKNOWN_FUNCTION),
    ]);
}

#[test]
fn truth() {
    check(&[
        ("$if($len(%title%),y,n)", "y"),
        ("$if($len(%album%),y,n)", "n"),
        ("[$add(1,2)]", ""),
        ("$if([%artist%],y,n)", "y"),
        ("$if($if(%album%,a,b),y,n)", "n"),
        ("[$puts(v,%artist%)]$if($get(v),y,n)", "y"),
        ("$puts(v,abc)$if($get(v),y,n)", "n"),
        ("[%artist%$null()]", "Daft Punk"),
    ]);
}

#[test]
fn evaluation_order_and_side_effects() {
    check(&[
        ("$if(%album%,$puts(v,a),$puts(v,b))$get(v)", "b"),
        ("$if(%artist%,$puts(v,a),$puts(v,b))$get(v)", "b"),
        ("$put(v,1)$put(v,$get(v)2)", "112"),
        ("$puts(V,x)$get(v)", "x"),
        ("$get(none)", ""),
    ]);
}

#[test]
fn control() {
    check(&[
        ("$if(%artist%,a,b)", "a"),
        ("$if(%artist%,a)", "[INVALID $IF SYNTAX]"),
        ("$if()", "[INVALID $IF SYNTAX]"),
        ("$if(1,2,3,4)", "[INVALID $IF SYNTAX]"),
        ("$if2(%album%,b)", "b"),
        ("$if2(%artist%,b)", "Daft Punk"),
        ("$if2(a,b,c)", "[INVALID $IF SYNTAX]"),
        ("$if3(%album%,%artist%,else)", "Daft Punk"),
        ("$if3(%album%,x,else)", "else"),
        ("$if3(only)", "only"),
        ("$if3()", ""),
        ("$decode(b,a,1,b,2,c,3)", "2"),
        ("$decode(z,a,1,def)", "def"),
        ("$decode(z,a,1)", ""),
        ("$decode(,,1,def)", "def"),
        ("$decode(ｂ,B,ok)", "ok"),
        ("$select(1,a,b)", "a"),
        ("$select(0,a,b)", "0"),
        ("$select(3,a,b)", ""),
        ("$select(-1,a,b)", ""),
        ("$iflonger(abc,2,y,n)", "y"),
        ("$iflonger(abc,3,y,n)", "n"),
        ("$iflonger(abc,2,y)", "[INVALID $IFLONGER SYNTAX]"),
        ("$ifgreater(10,9,y,n)", "y"),
        ("$ifgreater(9,9,y,n)", "n"),
        ("$ifgreater(-1,x,y,n)", "n"),
        ("$ifgreater(1,2,3)", "[INVALID $IFGREATER SYNTAX]"),
        ("$ifstrequal(ABC,abc,t)", "t"),
        ("$ifstrequal(abc,abd,t)", ""),
        ("$ifstrequal(,,t)", ""),
        ("$ifstrequal(%wide%,daft,t)", "t"),
        ("$ifstrnotequal(a,b,t)", "t"),
        ("$ifstrnotequal(a,A,t)", ""),
        ("$ifstrnotequal(,,t)", "t"),
        ("$ifstrequal2(a,A,t,e)", "t"),
        ("$ifstrequal2(a,b,t,e)", "e"),
    ]);
}

#[test]
fn text() {
    check(&[
        ("$len(%artist%)", "9"),
        ("$len(ünï)", "3"),
        ("$len()", "0"),
        ("$longest(ab,cd,e)", "ab"),
        ("$longest(a,bcd,efg)", "bcd"),
        ("$shortest(ab,c,d)", "c"),
        ("$longest()", ""),
        ("$left(abc,2)", "ab"),
        ("$cut(abc,-1)", "abc"),
        ("$left(äbc,1)", "ä"),
        ("$right(abc,2)", "bc"),
        ("$right(abc,9)", "abc"),
        ("$right(abc,-1)", "abc"),
        ("$right(abc,0)", ""),
        ("$pad(ab,4)", "ab  "),
        ("$pad(ab,7,xy)", "abxyxyx"),
        ("$pad(abcd,2)", "abcd"),
        ("$pad(ab,4,)", "ab  "),
        ("$lpad(ab,4,0)", "00ab"),
        ("$padcut(abcdef,3)", "abc"),
        ("$padcut(ab,3,-)", "ab-"),
        ("$repeat(ab,3)", "ababab"),
        ("$repeat(ab,-3)", ""),
        ("$trim(  a b  )", "a b"),
        ("$upper(straße)", "STRASSE"),
        ("$lower(ÄB)", "äb"),
        ("$caps(hELLO wORLD-wide)", "Hello World-Wide"),
        ("$caps2(hELLO wORLD)", "HELLO WORLD"),
        ("$abbr(Final Fantasy VI Advance,10)", "FFVIA"),
        ("$abbr(Short,10)", "Short"),
        ("$abbr(Street Fighter 2 RPG)", "SF2R"),
        ("$abbr(Mega RPG III)", "MRPGI"),
        ("$abbr('  the (big) one ')", "tbo"),
        ("$strchr(abcabc,c)", "3"),
        ("$strlchr(abcabc,cx)", "3"),
        ("$strrchr(abcabc,b)", "5"),
        ("$strchr(abc,z)", "0"),
        ("$strchr(abc,)", "0"),
        ("$strstr(ébcd,cd)", "3"),
        ("$strstr(abc,x)", "0"),
        ("$strstr(abc,)", "0"),
        ("$substr(abcdef,2,4)", "bcd"),
        ("$substr(abcdef,2)", "b"),
        ("$substr(abcdef,0,2)", "ab"),
        ("$substr(abcdef,5,99)", "ef"),
        ("$substr(abcdef,4,2)", ""),
        ("$split(a-b-c,-,0)", "a"),
        ("$split(a-b-c,-,1)", "b-c"),
        ("$split(abc,-,0)", "abc"),
        ("$split(abc,-,1)", ""),
        ("$split(abc,,0)", "abc"),
        ("$replace(aXbXc,X,--)", "a--b--c"),
        ("$replace(aaa,aa,b)", "ba"),
        ("$replace(abc,,x)", "abc"),
        ("$replace(abc,B,x)", "abc"),
        ("$char(65)$char(0)$char(-1)$char(1114112)$char(233)", "Aé"),
        ("$tab()", ""),
        ("$len($tab(3))", "3"),
        ("$len($crlf())", "2"),
    ]);
}

#[test]
fn numbers() {
    check(&[
        ("$num(5,3)", "005"),
        ("$num(-5,3)", "-05"),
        ("$dec(12345,2)", "12345"),
        ("$num(  +42x,0)", "42"),
        ("$num(abc,2)", "00"),
        ("$num(99999999999,0)", "2147483647"),
        ("$hex(255,4)", "00ff"),
        ("$hex(-1,0)", "ffffffff"),
        ("$add(1,2,3)", "6"),
        ("$add()", "0"),
        ("$add(2147483647,1)", "-2147483648"),
        ("$sub(10,3,2)", "5"),
        ("$mul(2,3,4)", "24"),
        ("$mul(65536,65536)", "0"),
        ("$div(100,3,0,2)", "16"),
        ("$div(-7,2)", "-3"),
        ("$div(-2147483648,-1)", "-2147483648"),
        ("$mod(17,5)", "2"),
        ("$mod(17,0,5)", "2"),
        ("$mod(-7,3)", "-1"),
        ("$muldiv(10,3,4)", "8"),
        ("$muldiv(-7,1,2)", "-4"),
        ("$muldiv(5,5,0)", "-1"),
        ("$muldiv(2147483647,2147483647,1)", "-1"),
        ("$muldiv(2147483647,2147483647,2147483647)", "2147483647"),
        ("$min(3,-1,2)", "-1"),
        ("$max(3,-1,2)", "3"),
        ("$max()", "0"),
    ]);
}

#[test]
fn paths() {
    check(&[
        ("$filepart(%filename%)", "c"),
        ("$filename(%filename%)", "c"),
        ("$fileext(%filename%)", "mp3"),
        ("$ext(/x/y.tar.gz)", "gz"),
        ("$directory(%filename%)", "b"),
        ("$directory(%filename%,2)", "a"),
        ("$pathltrim(%filename%)", "a\\b\\c.mp3"),
        ("$pathltrim(%filename%,2)", "b\\c.mp3"),
        ("$pathlpart(%filename%,2)", "C:\\a"),
        ("$pathrtrim(%filename%)", "C:\\a\\b"),
        ("$pathrpart(%filename%)", "c.mp3"),
        ("$pathrpart(%filename%,2)", "b\\c.mp3"),
        ("$pathsafe('a\\b|c*\"d\"<e>')", "a-b_c_'d'(e)"),
        ("$pathsafe(a$char(7)b)", "ab"),
    ]);
}

#[test]
fn time() {
    check(&[
        ("$systime_year()", "2026"),
        ("$systime_month()-$systime_day()", "03-07"),
        (
            "$systime_hour():$systime_minute():$systime_second()",
            "09:05:04",
        ),
    ]);
    let year: i32 = Formatter::new("$systime_year()")
        .format(&fields())
        .parse()
        .unwrap();
    assert!(year >= 2024);
}

#[test]
fn derived_fields() {
    check(&[
        ("%tracknumber%", "3/14"),
        ("%folder%", "b"),
        ("$num(%tracknumber%,2)", "03"),
    ]);
}

#[test]
fn custom_source() {
    struct Upper;
    impl FieldSource for Upper {
        fn field(&self, name: &str) -> Option<String> {
            Some(name.to_uppercase())
        }
    }
    assert_eq!(nulamp_tf::format("%Title% [%x%]", &Upper), "TITLE X");
}

#[test]
fn formatter_reuse() {
    let fmt = Formatter::default();
    let mut m = HashMap::new();
    m.insert("filename".to_string(), "/x/My Song.ogg".to_string());
    assert_eq!(fmt.format(&m), "My Song");
    m.insert("artist".to_string(), "Me".to_string());
    assert_eq!(fmt.format(&m), "Me - My Song");
    m.insert("title".to_string(), "T".to_string());
    assert_eq!(fmt.format(&m), "Me - T");
}

#[test]
fn output_is_capped() {
    assert_eq!(f("$repeat(x,2147483647)").len(), MAX_LEN);
    assert_eq!(f("$pad(x,2147483647)").len(), MAX_LEN);
    assert_eq!(
        f("$repeat($repeat(ab,100000),100000)$repeat(c,10)").len(),
        MAX_LEN
    );
    assert_eq!(f("$len($tab(2147483647))"), MAX_LEN.to_string());
}
