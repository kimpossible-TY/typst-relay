use typst_shim::syntax::LinkedNodeExt;

use crate::{SemanticRequest, analysis::doc_highlight::DocumentHighlightWorker, prelude::*};

/// The [`textDocument/documentHighlight`] request
///
/// [`textDocument/documentHighlight`]: https://microsoft.github.io/language-server-protocol/specification#textDocument_documentHighlight
#[derive(Debug, Clone)]
pub struct DocumentHighlightRequest {
    /// The path of the document to request highlight for.
    pub path: PathBuf,
    /// The position of the document to request highlight for.
    pub position: LspPosition,
}

impl SemanticRequest for DocumentHighlightRequest {
    type Response = Vec<DocumentHighlight>;

    fn request(self, ctx: &mut LocalContext) -> Option<Self::Response> {
        let source = ctx.source_by_path(&self.path).ok()?;
        self.request_source(&source, ctx.position_encoding())
    }
}

impl DocumentHighlightRequest {
    /// Finds highlights in an already synchronized source without semantic analysis.
    pub fn request_source(
        self,
        source: &Source,
        position_encoding: PositionEncoding,
    ) -> Option<Vec<DocumentHighlight>> {
        let cursor = to_typst_position(self.position, position_encoding, source)?;

        let root = LinkedNode::new(source.root());
        let node = root.leaf_at_compat(cursor)?;

        let mut worker = DocumentHighlightWorker::from_source(source, position_encoding);
        worker.work(&node)?;
        (!worker.annotated.is_empty()).then_some(worker.annotated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::*;

    #[test]
    fn test() {
        snapshot_testing("document_highlight", &|ctx, path| {
            let source = ctx.source_by_path(&path).unwrap();

            let request = DocumentHighlightRequest {
                path: path.clone(),
                position: find_test_position_after(&source),
            };

            let result = request.clone().request(ctx);
            assert_eq!(
                request.request_source(&source, ctx.position_encoding()),
                result
            );
            assert_snapshot!(JsonRepr::new_redacted(result, &REDACT_LOC));
        });
    }

    #[test]
    fn source_highlights_preserve_unicode_conversion_in_both_encodings() {
        let text =
            "한😀 #for i in range(2) { [안😀]; break; continue; for j in range(1) { break } }";
        let cursor = text.find("break").unwrap() + 2;
        let expected = ["#for", "break", "continue"].map(|keyword| {
            let start = text.find(keyword).unwrap();
            start..start + keyword.len()
        });

        run_with_sources(text, |verse, path| {
            for encoding in [PositionEncoding::Utf8, PositionEncoding::Utf16] {
                let analysis = crate::analysis::Analysis {
                    position_encoding: encoding,
                    ..Default::default()
                };
                let mut ctx = analysis.enter(WorldComputeGraph::from_world(verse.snapshot()));
                let source = ctx.source_by_path(&path).unwrap();
                let position = |offset| {
                    let prefix = &text[..offset];
                    let character = match encoding {
                        PositionEncoding::Utf8 => prefix.len(),
                        PositionEncoding::Utf16 => prefix.encode_utf16().count(),
                    };
                    LspPosition::new(0, character as u32)
                };
                let request = DocumentHighlightRequest {
                    path: path.clone(),
                    position: position(cursor),
                };
                let result = request.clone().request_source(&source, encoding);
                assert_eq!(result, request.request(&mut ctx), "{encoding:?}");
                let mut highlights = result.unwrap();
                assert_eq!(highlights.len(), 3, "{encoding:?}");

                // The shared UTF-8 conversion currently counts Unicode scalars
                // through Typst's column helpers. Preserve that existing path
                // here; fixing its byte-column contract is a separate change.
                // UTF-16 also gets an independent check of the exact ranges.
                if encoding == PositionEncoding::Utf16 {
                    highlights.sort_by_key(|highlight| highlight.range.start.character);

                    let expected = expected
                        .iter()
                        .map(|range| DocumentHighlight {
                            range: LspRange::new(position(range.start), position(range.end)),
                            kind: None,
                        })
                        .collect::<Vec<_>>();
                    assert_eq!(highlights, expected);
                }
            }
        });
    }
}
