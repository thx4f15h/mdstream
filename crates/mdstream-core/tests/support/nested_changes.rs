// Exact nested-candidate deltas, compared with the immutable a751b5a core.
// Original fixtures remain unchanged. Shortcut hashes include EVERY public
// snapshot/scalar field (without the old marker-restoration adapter). New
// parent retention also applies around failed angle atoms and footnotes.
// Work differences: O(1) journal splitting removes old Vec-copy counts;
// a retained parent journals one additional delimiter at a failed header.
#[allow(dead_code)]
fn hash(source: &str) -> u64 {
    source.bytes().fold(0xcbf29ce484222325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    })
}
#[allow(dead_code)]
pub fn shortcut(source: &str, original: u64) -> Option<u64> {
    SHORTCUT
        .iter()
        .find(|&&(s, old, _)| s == source && old == original)
        .map(|&(_, _, new)| new)
}
#[allow(dead_code)]
pub fn work(source: &str, original: u64) -> u64 {
    let source = hash(source);
    WORK.iter()
        .find(|&&(s, old, _)| s == source && old == original)
        .map_or(original, |&(_, _, new)| new)
}
const SHORTCUT: &[(&str, u64, u64)] = &[
    (
        "![foo ![bar](/url)](/url2)\n",
        0x6ed2790bdd8da92f,
        0x16ff58f56cd0e699,
    ),
    (
        "![foo [bar](/url)](/url2)\n",
        0x3711aed669bfcdfc,
        0x0b1d41dfb261da6f,
    ),
    ("[<x:a>]", 0xb7c773c9f4e9e399, 0xba99da68ba366efc),
    ("[<x:a>](u)", 0x9140c305b0984199, 0x137db03fcc84031a),
    ("[<x:a>][b]", 0x399411e1f73cf9e2, 0x06db890b7d5f925f),
    ("[<x:a>][]", 0x567b086a9b3fd90f, 0x00925b87dec8ccdc),
    (
        "[<x:a>](u '&amp;'&amp;)",
        0x748166e9bc4d85d8,
        0xab1024ebaa2790f7,
    ),
    ("[<x:a>]|c\n-|-\n", 0x4d5487db59f27d5a, 0x450b27fbb52c18fd),
    ("[<x:a>]\nwrong\n", 0xce5244f9b99920cd, 0xe815339cce3363ca),
    (
        "[<x:a>]\n\n[a]: /u\n",
        0x3469d863efc8155d,
        0xa352df63f483eb65,
    ),
    ("[[^f]](u)", 0xd08f479d81bd53a6, 0xa14276367a115384),
    ("[[^f]][b]", 0x45b6d0cc7ec8346a, 0xb9efc180d8798701),
    ("[![a](u)](u)", 0x352aeb9cc3e87b5c, 0x62998e8e827047e8),
    ("[![a](u)][b]", 0xc8722dd7893d8984, 0x4ca6ad0d25c0e0e4),
    ("> [<x:a>]", 0xb1df2eb42c30539f, 0xeb0617673749ce5e),
    ("> [<x:a>](u)", 0x18812b0b64e75cdd, 0x97f25e13d5da4c73),
    ("> [<x:a>][b]", 0x28a930c4111e0619, 0xaaddf5bf9e22d8d4),
    ("> [<x:a>][]", 0x11286c4acd9f5c13, 0xf8f2ab9748bd90e1),
    (
        "> [<x:a>](u '&amp;'&amp;)",
        0x04007127b9a37a9c,
        0x68445bf568b4afcf,
    ),
    ("> [<x:a>]|c\n-|-\n", 0x00afc43dea7db4ff, 0xc577ed99eba08cf0),
    ("> [<x:a>]\nwrong\n", 0x279aba8b04f09f77, 0x7aa8a70f972fe270),
    (
        "> [<x:a>]\n\n[a]: /u\n",
        0x87d4fa9433d8d68d,
        0xd3fd782d931f44d1,
    ),
    ("> [[^f]](u)", 0xa53bde48a3ded8ca, 0x5e9a51d06e80665f),
    ("> [[^f]][b]", 0x1d39d61d2225d062, 0x157acd45a0ffc370),
    ("> [![a](u)](u)", 0xe0dc221e9218cc85, 0x5703cdf81821675d),
    ("> [![a](u)][b]", 0xf6aa4675a86bb5a0, 0xbc7a181e81b222a3),
    ("- [<x:a>]", 0x6105a92d062cb877, 0x75a6467b18785986),
    ("- [<x:a>](u)", 0xe070fff16c8ee455, 0xbed6ead14f700c0f),
    ("- [<x:a>][b]", 0x1568e9afd9ad9614, 0x76d88dd1fb5cbe2b),
    ("- [<x:a>][]", 0x57ef84d412e26686, 0x99fb01d9dd4c5618),
    (
        "- [<x:a>](u '&amp;'&amp;)",
        0x574227f716092a30,
        0x064679bd8d6ac087,
    ),
    ("- [<x:a>]|c\n-|-\n", 0xc06515a118b15ee8, 0x42ff3053d2eb6e87),
    ("- [<x:a>]\nwrong\n", 0x339473f6caba6268, 0x7aaf8d540c8a7c7b),
    (
        "- [<x:a>]\n\n[a]: /u\n",
        0xcfa869fb7f0faaf6,
        0x17d64054a80cce34,
    ),
    ("- [[^f]](u)", 0xb8e730446a41e3cd, 0xcfbe992bfd4b04e4),
    ("- [[^f]][b]", 0x59145e3977ba2cd4, 0x2bf5ef65116af6be),
    ("- [![a](u)](u)", 0x317472b857c8ee4b, 0xdc0fa010da2eabe5),
    ("- [![a](u)][b]", 0xeb4de682b116d6ad, 0xa3ffb43a525ef9cc),
    ("- >\t[<x:a>]", 0xa0fa6efdd2598bf3, 0x027189dcc9ad54b2),
    ("- >\t[<x:a>](u)", 0x26d8c2f4ebae2da7, 0x11b9681fc84c7388),
    ("- >\t[<x:a>][b]", 0x63fa2b89fdf93bf6, 0x33ebe51437fcd6b2),
    ("- >\t[<x:a>][]", 0x956ee648086dfb33, 0xcaf4649f403a36a4),
    (
        "- >\t[<x:a>](u '&amp;'&amp;)",
        0x87f458817c15d8a6,
        0xcd72759cc5d8b143,
    ),
    (
        "- >\t[<x:a>]|c\n-|-\n",
        0x6abe9f3a60b1e76f,
        0x6735937df4d42842,
    ),
    (
        "- >\t[<x:a>]\nwrong\n",
        0xbcbf85f2ee950033,
        0x0068bfb2d8d44f26,
    ),
    (
        "- >\t[<x:a>]\n\n[a]: /u\n",
        0x317222f79f5fce24,
        0xa649df002b02c4c0,
    ),
    ("- >\t[[^f]](u)", 0xf70cb0b0ae7844ac, 0x40ece01c207eb800),
    ("- >\t[[^f]][b]", 0x1f522799e9a54b9a, 0xfc2975cce844692b),
    ("- >\t[![a](u)](u)", 0x128db5b9e59a939b, 0x6194f27c3e3741df),
    ("- >\t[![a](u)][b]", 0x12380529b563819b, 0x14c1ce6f2e2b6ffe),
    ("> -\t[<x:a>]", 0x0e993ab7eb40605a, 0x9d89d6b382edd793),
    ("> -\t[<x:a>](u)", 0x6067987bf4f0994a, 0xe383d55429c70baf),
    ("> -\t[<x:a>][b]", 0x24876885028f72df, 0x5ecacb0442257a0b),
    ("> -\t[<x:a>][]", 0x355ea4592f96e0e2, 0x39603e7c09b18bdd),
    (
        "> -\t[<x:a>](u '&amp;'&amp;)",
        0x3313044a1286a6b5,
        0x4c17625bbb50da10,
    ),
    (
        "> -\t[<x:a>]|c\n-|-\n",
        0x6faa3a2cee8d9d80,
        0xb55d8de9af4e1e3d,
    ),
    (
        "> -\t[<x:a>]\nwrong\n",
        0xc1fcd045257a7b2c,
        0x7d5589f2a1e05391,
    ),
    (
        "> -\t[<x:a>]\n\n[a]: /u\n",
        0xf62149032543c1f8,
        0xc8a34393c4702b1c,
    ),
    ("> -\t[[^f]](u)", 0xbc22d82b98e4dab7, 0x538a91827c53ce75),
    ("> -\t[[^f]][b]", 0x3ec307a35876f087, 0x38464f99096ca3d2),
    ("> -\t[![a](u)](u)", 0x0dc8360d9ad2b008, 0xd73fe184a2bb9aaa),
    ("> -\t[![a](u)][b]", 0xd25a18896ed827de, 0x644489da983d59b9),
    ("# [<x:a>]", 0xbaf6d939203bfa65, 0xeaba4727edab30cc),
    ("# [<x:a>](u)", 0x89c66a3dfcffe3f5, 0xaa465a680888faab),
    ("# [<x:a>][b]", 0xc622595c9c0c8c1c, 0xa79ab9fe68efa659),
    ("# [<x:a>][]", 0xce7bf2db45ab0df0, 0x323182203fee4664),
    (
        "# [<x:a>](u '&amp;'&amp;)",
        0xcade2ed735f9e764,
        0x3a8f888060e3abf7,
    ),
    ("# [<x:a>]|c\n-|-\n", 0x5724d310c6dde5be, 0xfc3cd577f034effb),
    ("# [<x:a>]\nwrong\n", 0x263c73f4e9f4d63d, 0x131470b4f1c769de),
    (
        "# [<x:a>]\n\n[a]: /u\n",
        0xd2b6ca665708ebf5,
        0x207f4834002250d1,
    ),
    ("# [[^f]](u)", 0x7f62a6f9dbc75837, 0x2b046879e662c148),
    ("# [[^f]][b]", 0x373a968b0402558a, 0x1d3e9d33aafa6524),
    ("# [![a](u)](u)", 0x940ad8e852027581, 0x0d4876d1b6b4df91),
    ("# [![a](u)][b]", 0x1c4c340b425469bd, 0xafc951ffa1cd81da),
    ("| [<x:a>]", 0x917a17351e1e032f, 0xf146eb3b3742119c),
    ("| [<x:a>](u)", 0x0e3d26e937baf38b, 0xc9971e43abb070a8),
    ("| [<x:a>][b]", 0x2741e2b39b7f5f44, 0x6b8934e2570213a7),
    ("| [<x:a>][]", 0x06d0ec88eeb5e9ea, 0xa565fe7d21d0f76e),
    (
        "| [<x:a>](u '&amp;'&amp;)",
        0x682641b22c66bf4a,
        0x5e43e61e2db42a43,
    ),
    ("| [<x:a>]|c\n-|-\n", 0x9594debc55545057, 0xeea9a946816b5424),
    ("| [<x:a>]\nwrong\n", 0xd4ba255c6c59581f, 0xdbc8e7784cf803d6),
    (
        "| [<x:a>]\n\n[a]: /u\n",
        0x8d6b233cc7def21d,
        0xc60215f569707adb,
    ),
    ("| [[^f]](u)", 0x60977544d8c3ed23, 0xfdde2f6f527b448b),
    ("| [[^f]][b]", 0x48828a57c86e612c, 0xa0f85afd510b5b3e),
    ("| [![a](u)](u)", 0x33a64bf627a57281, 0x43b59ced15fb92ea),
    ("| [![a](u)][b]", 0x81433c28f546ab69, 0xa4e840ae35d75dfc),
];
const WORK: &[(u64, u64, u64)] = &[
    (0x08fe0aea23624ac3, 0xa4ff5a1e8ae6cb85, 0x13ef890f56d08f42),
    (0x08fe0aea23624ac3, 0xbb3b50202733f3ec, 0x2a0b1ce79c6d63ce),
    (0x1bb7f40f9ad941a8, 0x36662efaa94cc9e5, 0x89e279144d276d30),
    (0x1bb7f40f9ad941a8, 0xb4af315994d87ced, 0x86a99636c0193641),
    (0x24e14b76de39383d, 0x61b9f234c3bdea33, 0x0afba99068c4a294),
    (0x24e14b76de39383d, 0x76c3216b0da4c104, 0xa235aa81dd961d5d),
    (0x28100676c26daa01, 0xa66bd071827de83a, 0xc4933471ad4a75dd),
    (0x28100676c26daa01, 0xf59aa34b139fd4d3, 0x087981aed119cb1a),
    (0x293860cb760995bf, 0x127b8f477f1de5eb, 0x74aadfedf4f71c02),
    (0x293860cb760995bf, 0x8fa431b76e31a85b, 0x8b52181d02c70b27),
    (0x31726515ba6c2f0d, 0x6d48b1c7b77f138f, 0x4a7062a28b42feda),
    (0x31726515ba6c2f0d, 0x7356885babccb9ff, 0x8c4b7692e87783b9),
    (0x3373abf40fd6028c, 0x4aebd5c69367761c, 0x2666d1ef1ccc967b),
    (0x337ec21e7bd98079, 0xb7777fe019a04b5f, 0x45fbd77eb12bd94b),
    (0x337ec21e7bd98079, 0xf5a60f31e480a1b2, 0x46d97b38bcef7455),
    (0x43987bf1c14d1dcf, 0x1b42910f53d5f24c, 0x0b0e7db2a7fc2ec8),
    (0x43987bf1c14d1dcf, 0x8b6074ced24b8998, 0xfc707e49f1ca4315),
    (0x5e775ba297026ee7, 0x093e5761f466c09a, 0xe5257a1ab15a230b),
    (0x5e775ba297026ee7, 0x2640e3edaafef756, 0x7014aedb10739c92),
    (0x7190caebc5d3e8e5, 0x888fc2163561c001, 0x6d9e3f98f357017b),
    (0x7190caebc5d3e8e5, 0xf37669a71ea458a9, 0x3faedad175f71ee4),
    (0x7dc2840fe647afce, 0xd0eccc6512a2ded0, 0x7d6c95fac8ef8999),
    (0x7dc2840fe647afce, 0xe37e0cd691db6760, 0xc421337ddb0ebc5e),
    (0x873fa24699227cb7, 0xaa647b6dba706222, 0x0916ff8817febea3),
    (0x873fa24699227cb7, 0xfcce36af6cdb1b96, 0x6383ddd4e586db02),
    (0x8f5665081aeb955e, 0x48ebd7e5a7e36b10, 0xa9a17b566069e626),
    (0x8f5665081aeb955e, 0xd26268ecd16aeb18, 0x0ddac3683052e201),
    (0x969cd19045e4cc88, 0x7e72546907470f58, 0x2e5b3c643d174575),
    (0x969cd19045e4cc88, 0xc76db8d67c1cd333, 0x420b6a777fcb48b7),
    (0xb5524efed7747a20, 0x7eec225be65ca763, 0x24b1af60fb37b45e),
    (0xb5524efed7747a20, 0xc20d26824f0ed4e3, 0x6f6ba461b3eca511),
    (0xb61f2d2885d5f306, 0xc618ace87e0f13fd, 0xbbc9412eb8487ca5),
    (0xc612c7e652a4411c, 0x9899d8faf4af7ba4, 0x11967353a3cffd6f),
    (0xcf13b40a0682feae, 0xbee6cb13b8092128, 0xa4cc52879532b1d1),
    (0xd1449a26c3c780e6, 0x214cabfbfef31108, 0x9e5d7d045e2ae784),
    (0xd1449a26c3c780e6, 0x4b42bb064bfa7e43, 0x8d110abeffc59d56),
    (0xd9061dbe1fb85df0, 0x4e9963c83f6f8000, 0x7440239690cb86ea),
    (0xd9061dbe1fb85df0, 0xd0a3705744453614, 0xe2c98c9c8acb1ce5),
    (0xe40b71fb9b3a8575, 0xc199f34142b58a4c, 0xbf6175da65207ad9),
    (0xe40b71fb9b3a8575, 0xe409646f63b882e3, 0x7b63401fc64b68a4),
    (0xf2d558841f2a8646, 0x4e5c284ee55fa723, 0xa3e994c2b4b29dc6),
    (0xf2d558841f2a8646, 0x65de452b659cb6ab, 0xe5e7b0a25526a107),
    (0xf7e0eebe3c5d53f3, 0x37e43dcec3c53e0e, 0xea8229e38b627ea8),
    (0xf7e0eebe3c5d53f3, 0x8257d3e733a1dd34, 0x89754cb303b1848b),
    (0xfb73903f85063ac3, 0x36657b25579f7f35, 0xdb8d27ae5f1f8b41),
    (0xfb73903f85063ac3, 0xf8b2e306ec6e7116, 0x5cb59b2a072651b3),
];
