# Prolly Tree Blog Cover Prompt

- Aspect ratio: 5:2
- Output: `docs/concepts/cover/prolly-tree-cover.png`
- Current source: `docs/concepts/cover/prolly-tree-cover.svg`
- Current render path: SVG rendered to PNG via `rsvg-convert`
- Previous annotated-review backup: `docs/concepts/cover/prolly-tree-cover-before-annotation-fix.svg`
- Previous concept backup: `docs/concepts/cover/prolly-tree-cover-concept-v1.svg`
- Previous 3D version-axis backup: `docs/concepts/cover/prolly-tree-cover-3d-version-axis.svg`
- Previous 2D structural backup: `docs/concepts/cover/prolly-tree-cover-2d-structure.svg`
- Previous AI-generated bitmap backup: `docs/concepts/cover/prolly-tree-cover-ai-v1.png`
- Previous source image: `/Users/haipingfu/.codex/generated_images/019f1132-1e01-7ce2-aff9-61b02319b36a/ig_08363971c1328a0a016a420f9faf2081948597ac3311f1c3bd.png`

## Current Diagram Intent

The current cover is a refined concept SVG. It avoids low-level node diagrams and uses a balanced three-card composition:

- B-tree-like structure: ordered routing over key ranges.
- Stable boundaries: content-defined chunks preserve structure under inserts.
- Multi-version tree: v1, v2, and v3 each have their own root CID; each update creates a new root and rebuilds only the changed leaf-to-root path, while unchanged side subtrees are reused by content identity.

All three ideas converge into one bottom message: a root CID is a compact version handle over a shared content-addressed graph.

## Annotated Review Fixes

- Stable boundary chunks use variable widths rather than fixed-size blocks.
- The B-tree-like structure card includes root, internal nodes, and leaf ranges with visible spacing.
- The multi-version card is now tree-shaped across v1 -> v2 -> v3: root A points to the old path, root B points to Δ1, and root C points to Δ2; all three roots reuse the same green shared subtree where CIDs are unchanged.
- Card-to-summary connector lines originate from card bottoms and no longer appear to start from the wrong internal element.
- The multi-version card now emphasizes the path-copying rule: the delta creates new CIDs only for the changed leaf, its parent/internal path, and the root; sibling subtrees keep the same CIDs and are structurally shared.
- Tree arrows use a smaller local marker and short, edge-aligned paths so arrowheads do not collide with version dots, root boxes, or delta boxes.

## Previous AI Prompt

Create a professional technical blog cover image in a 5:2 aspect ratio. Topic: "Prolly Trees: Content-Addressed Ordered Indexes for Versioned Data". Dark editorial tech style, crisp digital illustration, no realistic people. Visual concept: a luminous B+tree-like ordered index made of small node blocks, leaf ranges at the bottom, internal routing nodes above, and a glowing root CID at the top. Add subtle Merkle graph connections, hash/CID motifs, content-defined chunk boundaries, and a sense of versioned snapshots branching and sharing unchanged subtrees. Palette: deep navy/slate background, cyan and violet tree nodes, emerald highlights for shared subtrees, small amber accents for hash boundaries. Composition: wide cinematic 5:2 banner, strong central visual anchor, generous negative space, clean professional engineering-blog aesthetic. Include readable title text: "Prolly Trees" and smaller subtitle: "Content-Addressed Ordered Indexes for Versioned Data". Avoid clutter, avoid dense small labels, avoid code screenshots, avoid stock-photo look.
