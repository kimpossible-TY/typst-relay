/// path: nested-import-leaf.typ
#let theorem(body) = body
-----
/// path: nested-import-branch.typ
#import "nested-import-leaf.typ" as corners
-----
/// path: nested-import-root.typ
#import "nested-import-branch.typ" as presets
-----
#import "nested-import-root.typ" as theoretic
#import theoretic.presets.corners: theorem
#(/* position after */ theorem)
