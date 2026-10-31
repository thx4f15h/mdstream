use mdtui3::{Block, Document, LinkTarget, Reference, Span, Style, TileId};

#[derive(Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub source: String,
    pub blocks: Vec<Block>,
    pub containers: Vec<mdtui3::Container>,
    pub tiles: Vec<Span>,
    pub references: Vec<Reference>,
    pub links: Vec<LinkTarget>,
    pub link_info: Vec<mdtui3::LinkInfo>,
    pub headings: Vec<mdtui3::Heading>,
    pub tables: Vec<mdtui3::Table>,
    pub maths: Vec<mdtui3::Math>,
    pub math_payloads: Vec<Vec<(Span, String)>>,
    pub math_block_payloads: Vec<Vec<(Span, String)>>,
    pub footnotes: Vec<mdtui3::Footnote>,
    pub footnote_payloads: Vec<Vec<(Span, String)>>,
    pub footnote_references: Vec<mdtui3::FootnoteReference>,
    pub runs: Vec<(Span, Option<mdtui3::BlockId>, Style, Option<mdtui3::LinkId>)>,
    pub display: Vec<(Span, Option<Span>, String)>,
}

pub fn snapshot(doc: &Document) -> Snapshot {
    Snapshot {
        source: doc.source().to_owned(),
        blocks: doc.blocks().to_vec(),
        containers: (0..doc.container_count())
            .map(|i| doc.container(mdtui3::ContainerId(i)).unwrap())
            .collect(),
        tiles: (0..doc.tile_count())
            .map(|i| doc.tile(TileId(i)).unwrap())
            .collect(),
        references: doc.references().to_vec(),
        links: doc.links().to_vec(),
        link_info: (0..doc.links().len())
            .map(|i| *doc.link_info(mdtui3::LinkId(i)).unwrap())
            .collect(),
        headings: doc.headings().entries().to_vec(),
        tables: doc.tables().to_vec(),
        maths: doc.maths().to_vec(),
        math_block_payloads: doc
            .blocks()
            .iter()
            .enumerate()
            .filter_map(|(i, _)| {
                let mut cursor = doc.math_block_payload(mdtui3::BlockId(i))?;
                let mut budget = mdtui3::Budget::new(mdtui3::WorkLimit::unlimited());
                let mut chunks = Vec::new();
                while let Some(chunk) = cursor.advance(&mut budget).unwrap() {
                    chunks.push((chunk.source, chunk.text.to_owned()));
                }
                Some(chunks)
            })
            .collect(),
        math_payloads: (0..doc.maths().len())
            .map(|i| {
                let mut cursor = doc.math_payload(mdtui3::MathId(i)).unwrap();
                let mut budget = mdtui3::Budget::new(mdtui3::WorkLimit::unlimited());
                let mut chunks = Vec::new();
                while let Some(chunk) = cursor.advance(&mut budget).unwrap() {
                    chunks.push((chunk.source, chunk.text.to_owned()));
                }
                chunks
            })
            .collect(),
        footnotes: doc.footnotes().to_vec(),
        footnote_payloads: (0..doc.footnotes().len())
            .filter_map(|i| {
                let mut cursor = doc.footnote_payload(mdtui3::FootnoteId(i))?;
                let mut budget = mdtui3::Budget::new(mdtui3::WorkLimit::unlimited());
                let mut chunks = Vec::new();
                while let Some(chunk) = cursor.advance(&mut budget).unwrap() {
                    chunks.push((chunk.source, chunk.text.to_owned()));
                }
                Some(chunks)
            })
            .collect(),
        footnote_references: doc.footnote_references().to_vec(),
        runs: doc
            .runs_in(Span::new(0, doc.source().len()))
            .unwrap()
            .map(|r| (r.source, r.block, r.style, r.link))
            .collect(),
        display: doc
            .display_runs_in(Span::new(0, doc.source().len()))
            .unwrap()
            .map(|r| (r.source, r.replacement, r.text.to_owned()))
            .collect(),
    }
}
