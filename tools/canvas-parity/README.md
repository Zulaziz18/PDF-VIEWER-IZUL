# Canvas ↔ PDFium parity

The other half of SPEC 3.3's contract, and the half a headless CI cannot run.

`crates/izul-pdf/src/golden.rs` proves that the **appearance-stream** backend
renders to a known picture, for every annotation kind. This proves that the
**canvas** backend — the live proxy the user sees while dragging — draws the
same display list to the same picture.

It is not part of `npm test` and not part of CI, on purpose: it needs a real
Chromium and a real PDFium, and a test that silently skips in the environment
where it matters is worse than one that has to be asked for. Run it by hand, and
quote the number.

```bash
cargo test -p izul-pdf --lib golden          # regenerate baselines + display lists
node tools/canvas-parity/run.mjs             # draw them on a canvas and compare
```

## What the numbers mean

The threshold here is **not** SPEC 3.3's 0.5 %, and the difference matters.

At rest, the picture on screen *is* PDFium's render of the appearance stream —
the proxy is replaced (SPEC 3.3), so at-rest parity is structural and the golden
test is what guards it. The canvas proxy only has to be close enough that the
swap is not visible mid-gesture, and it is drawn by a different rasteriser: a
browser's antialiasing, its stroke joins and its hinting are not PDFium's, and
demanding pixel equality would be demanding two rasterisers be one program.

So this harness answers a different question — *is the proxy the same drawing* —
and the number it reports is the fraction of pixels differing by more than a
channel tolerance. Anything that would be visible as a **wrong shape** (a
missing arrow head, a mirrored y axis, a curve smoothed differently) moves it
far past any sane threshold; antialiasing alone does not.
