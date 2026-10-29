// CommonMark 0.31.2, examples 594–612, John MacFarlane.
// Source: https://spec.commonmark.org/0.31.2/spec.json
// License: CC BY-SA 4.0 https://creativecommons.org/licenses/by-sa/4.0/
// URL percent encoding is normalized for this source-preserving core.
pub const AUTOLINKS: &[(usize, &str, Option<&str>, &str, bool)] = &[
    (594, r###"<http://foo.bar.baz>
"###, Some(r###"http://foo.bar.baz"###), r###"http://foo.bar.baz"###, true),
    (595, r###"<https://foo.bar.baz/test?q=hello&id=22&boolean>
"###, Some(r###"https://foo.bar.baz/test?q=hello&id=22&boolean"###), r###"https://foo.bar.baz/test?q=hello&id=22&boolean"###, true),
    (596, r###"<irc://foo.bar:2233/baz>
"###, Some(r###"irc://foo.bar:2233/baz"###), r###"irc://foo.bar:2233/baz"###, true),
    (597, r###"<MAILTO:FOO@BAR.BAZ>
"###, Some(r###"MAILTO:FOO@BAR.BAZ"###), r###"MAILTO:FOO@BAR.BAZ"###, true),
    (598, r###"<a+b+c:d>
"###, Some(r###"a+b+c:d"###), r###"a+b+c:d"###, true),
    (599, r###"<made-up-scheme://foo,bar>
"###, Some(r###"made-up-scheme://foo,bar"###), r###"made-up-scheme://foo,bar"###, true),
    (600, r###"<https://../>
"###, Some(r###"https://../"###), r###"https://../"###, true),
    (601, r###"<localhost:5001/foo>
"###, Some(r###"localhost:5001/foo"###), r###"localhost:5001/foo"###, true),
    (602, r###"<https://foo.bar/baz bim>
"###, None, r###"<https://foo.bar/baz bim>"###, true),
    (603, r###"<https://example.com/\[\>
"###, Some(r###"https://example.com/\[\"###), r###"https://example.com/\[\"###, true),
    (604, r###"<foo@bar.example.com>
"###, Some(r###"mailto:foo@bar.example.com"###), r###"foo@bar.example.com"###, true),
    (605, r###"<foo+special@Bar.baz-bar0.com>
"###, Some(r###"mailto:foo+special@Bar.baz-bar0.com"###), r###"foo+special@Bar.baz-bar0.com"###, true),
    (606, r###"<foo\+@bar.example.com>
"###, None, r###"<foo+@bar.example.com>"###, false),
    (607, r###"<>
"###, None, r###"<>"###, true),
    (608, r###"< https://foo.bar >
"###, None, r###"< https://foo.bar >"###, true),
    (609, r###"<m:abc>
"###, None, r###"<m:abc>"###, true),
    (610, r###"<foo.bar.baz>
"###, None, r###"<foo.bar.baz>"###, true),
    (611, r###"https://example.com
"###, None, r###"https://example.com"###, true),
    (612, r###"foo@bar.example.com
"###, None, r###"foo@bar.example.com"###, true),
];
