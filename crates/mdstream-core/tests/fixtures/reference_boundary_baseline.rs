// Actual clean 1eaecbf predecessor; normalized only at the listed syntax bytes.
type BoundaryBaseline = (&'static str, &'static [(usize, usize)], u64);
const BASELINE: &[BoundaryBaseline] = &[
// 569
("[foo][bar][baz]\n\n[baz]: /url\n", &[(0, 1), (4, 10)], 0x1e8f105eff097c78),
// 571
("[foo][bar][baz]\n\n[baz]: /url1\n[foo]: /url2\n", &[(0, 1), (4, 10)], 0x6530e599e1710dad),
// definition-first
("[b]: /b\n\n[a][b]", &[], 0xf993f8a5b903ec08),
// full-blank-label
("[ ][b]", &[], 0x2c45ead3ea4fc635),
// quote-header-confirm
("> [a\n> b][c] | z\n> - | -\n", &[], 0x27086d7e014754ae),
// quote-header-fail
("> [a\n> b][c] | z\n> wrong\n", &[(2, 3), (8, 12)], 0x913996f8de316e5d),
// full-none
("[a][b]", &[(0, 1), (2, 6)], 0x5f29c93db96c2e04),
// full-only-a
("[a][b]\n\n[a]: /a\n", &[(0, 1), (2, 6)], 0xa1cf915845fa52f9),
// full-only-b
("[a][b]\n\n[b]: /b 'T'\n", &[], 0x85ce5a1ffab27bf8),
// collapsed-none
("[a][]", &[(0, 1), (2, 5)], 0x6ea4b7bab83097e7),
// collapsed-defined
("[a][]\n\n[a]: /a\n", &[], 0x111555f413539005),
// image-none
("![*a*][b]", &[(0, 2), (5, 9)], 0xa10bd2b58cf2f189),
// image-defined
("![*a*][b]\n\n[b]: /b\n", &[], 0x819724449a53868b),
// empty-full-link
("[][b]", &[], 0x1f65b6fec1eb6274),
// empty-full-image
("![][b]", &[], 0x55fd7fb7119de39f),
// empty-collapsed
("![][]", &[], 0x5cfc15305798c11b),
// empty-direct
("![](u 't')", &[], 0x3183b4fdb77ffcbf),
// direct
("[a](u 't')", &[], 0xe9171f420d11bf7f),
// shortcut
("![*a*]", &[], 0x4b0a6d066b183e8b),
// escape
("[a\\]][b\\]]", &[(0, 1), (4, 10)], 0x0d372865499434a1),
// unicode
("[中🙂][鍵]", &[(0, 1), (8, 14)], 0xfa7b48eb06c7cf27),
// opaque-suffix
("[a][*b*&amp;\\!]", &[(0, 1), (2, 15)], 0x7d0c524aa04c87a2),
// invalid-suffix
("[a][b[c]", &[], 0x871aa02332e915b9),
// invalid-space
("[a][ ]", &[], 0x1fd6825be763633b),
// unfinished
("[a][b", &[], 0xb84ebf8aca239f54),
// container
("- [*a*\n  b][c]\n", &[(2, 3), (10, 14)], 0x91ec2dd646660338),
// quote
(" > ![a][b]\n", &[(3, 5), (6, 10)], 0x6b5bfb618d769f7a),
// header-confirm
("[a\nb][c] | z\n- | -\n", &[], 0xc3346391f387fd1f),
// header-fail
("[a\nb][c] | z\nno\n", &[(0, 1), (4, 8)], 0x6bcf500739d33f63),
// header-eof
("[a\nb][c] | z", &[(0, 1), (4, 8)], 0x38edbf736a2707d8),
// atx
("# [a][b]\n", &[(2, 3), (4, 8)], 0xe9518fd2ab6130ab),
// structural-hidden
("> ![a][b]\n\n[b]: /b\n", &[], 0xd7ca22fd570a10c6),
// adjacent
("[a][b][c][d]", &[(0, 1), (2, 6), (6, 7), (8, 12)], 0x994bff8114308911),
// hardbreak
("[a\\\nb][c]", &[(0, 1), (5, 9)], 0x2cad7c10fe205c44),
// styles
("**x [*a*][b] y**", &[(4, 5), (8, 12)], 0xc8e16e7cd3ce64e8),
// rawmath
("[$ $][b]", &[(0, 1), (4, 8)], 0x3a08109829e54092),
// rawcode
("[` `][b]", &[(0, 1), (4, 8)], 0xb0acdbd64a85dfda),
// suffix-998
("[a][bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb]", &[(0, 1), (2, 1003)], 0xdc43ab741c0448d5),
// suffix-999
("[a][bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb]", &[(0, 1), (2, 1004)], 0x7d2df40da6f05886),
// suffix-1000
("[a][bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb]", &[], 0x171916b5da262612),
];
