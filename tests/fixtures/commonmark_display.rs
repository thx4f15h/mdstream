// CommonMark 0.31.2, selected display-text cases, John MacFarlane.
// Source: https://spec.commonmark.org/0.31.2/spec.json
// License: CC BY-SA 4.0 https://creativecommons.org/licenses/by-sa/4.0/
// HTML tags removed; terminal newlines excluded for comparison.
pub const DISPLAY_CASES: &[(usize, &str, &str)] = &[
    (25, r###"&nbsp; &amp; &copy; &AElig; &Dcaron;
&frac34; &HilbertSpace; &DifferentialD;
&ClockwiseContourIntegral; &ngE;
"###, r###"  & © Æ Ď
¾ ℋ ⅆ
∲ ≧̸"###),
    (26, r###"&#35; &#1234; &#992; &#0;
"###, r###"# Ӓ Ϡ �"###),
    (27, r###"&#X22; &#XD06; &#xcab;
"###, r###"" ആ ಫ"###),
    (28, r###"&nbsp &x; &#; &#x;
&#87654321;
&#abcdef0;
&ThisIsNotDefined; &hi?;
"###, r###"&nbsp &x; &#; &#x;
&#87654321;
&#abcdef0;
&ThisIsNotDefined; &hi?;"###),
    (29, r###"&copy
"###, r###"&copy"###),
    (30, r###"&MadeUpEntity;
"###, r###"&MadeUpEntity;"###),
    (35, r###"`f&ouml;&ouml;`
"###, r###"f&ouml;&ouml;"###),
    (36, r###"    f&ouml;f&ouml;
"###, r###"f&ouml;f&ouml;"###),
    (37, r###"&#42;foo&#42;
*foo*
"###, r###"*foo*
foo"###),
    (39, r###"foo&#10;&#10;bar
"###, r###"foo

bar"###),
    (40, r###"&#9;foo
"###, r###"	foo"###),
    (328, r###"`foo`
"###, r###"foo"###),
    (329, r###"`` foo ` bar ``
"###, r###"foo ` bar"###),
    (330, r###"` `` `
"###, r###"``"###),
    (331, r###"`  ``  `
"###, r###" `` "###),
    (332, r###"` a`
"###, r###" a"###),
    (333, r###"` b `
"###, r###" b "###),
    (334, r###"` `
`  `
"###, r###" 
  "###),
    (335, "``\nfoo\nbar  \nbaz\n``\n", r###"foo bar   baz"###),
    (336, "``\nfoo \n``\n", r###"foo "###),
    (337, "`foo   bar \nbaz`\n", r###"foo   bar  baz"###),
    (338, r###"`foo\`bar`
"###, r###"foo\bar`"###),
    (339, r###"``foo`bar``
"###, r###"foo`bar"###),
    (340, r###"` foo `` bar `
"###, r###"foo `` bar"###),
];
