# Performance & Known Issues

## Mobile gate freeze (after extended play)

**Symptom**: Game freezes for one frame when reaching a gate after playing for a while on mobile (WebGL2 / WASM). Confirmed on Sony phone; does not reproduce on Pixel or Samsung.

**Root cause — Text3d atlas upload**
Each gate spawn creates 3 `Text3d` entities (1 kanji crossbeam + 2 hiragana signs). `bevy_rich_text3d` rasterizes each new glyph into the `TextAtlas` texture and then uploads the updated texture to the GPU. On mobile WebGL2, GPU texture uploads are synchronous and can stall a frame. The freeze occurs "after a while" rather than immediately because SM-2 cycles through all 30 words gradually, so some kanji are new-to-the-atlas later in the session.

**Fix (not yet implemented)**: Pre-warm the `TextAtlas` at startup by spawning all N5_WORDS as invisible off-screen `Text3d` entities for a couple of frames, then despawning them. This moves the rasterization cost to the loading screen.

**Secondary suspect — thermal throttling**
After extended mobile play, the phone throttles CPU/GPU clock speed. Gate spawn is the heaviest frame (entity creation + possible atlas work), so it's the most likely frame to visibly stutter under thermal pressure. Not fixable in code.

---

## SMAA on mobile WebGL2

`SmaaPreset::Medium` requires an extra render pass with intermediate framebuffers. Cheap on desktop; non-trivial overhead on mobile WebGL2. Does not explain the gate-specific freeze but contributes to general frame cost and thermal load.

**Fix (not yet implemented)**: Compile SMAA out of WASM builds with `#[cfg(not(target_arch = "wasm32"))]` around the `Smaa` component in `setup()`.

---

## Z-coordinate float drift (long sessions)

Player Z decreases continuously at 25 u/s. f32 precision starts degrading noticeably around z = ±100,000 (about 1 hour of play). Not a freeze cause but could produce visual jitter in very long sessions.

**Fix (not yet implemented)**: Periodically re-origin the world by shifting all entity Z positions to keep the player near zero. Standard technique for endless-runner games.
