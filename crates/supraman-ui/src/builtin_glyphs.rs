// Box drawing and block elements drawn from geometry, as Windows Terminal's
// src/renderer/atlas/BuiltinGlyphs.cpp (MIT): its shape table and its DrawBuiltinGlyph rounding.

use Shape::*;

const FIRST: u32 = 0x2500;
const COUNT: usize = 0xA0;

pub(crate) fn is_builtin(ch: char) -> bool {
    (ch as u32).wrapping_sub(FIRST) < COUNT as u32
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Shape {
    /// `░▒▓` as the foreground at this opacity, where Windows Terminal stipples.
    Shade(f32),
    Fill,
    Light,
    Heavy,
    Rect,
    Round,
}

/// A fraction of the cell plus a multiple of the light line width.
type Pos = (f32, f32);

const MIN: Pos = (-0.5, 0.);
const MAX: Pos = (1.5, 0.);
const P0_1: Pos = (0., 0.);
const P1_1: Pos = (1., 0.);
const P1_2: Pos = (0.5, 0.);
const P1_2_SUB_0_5: Pos = (0.5, -0.5);
const P1_2_ADD_0_5: Pos = (0.5, 0.5);
const P1_2_SUB_1: Pos = (0.5, -1.);
const P1_2_ADD_1: Pos = (0.5, 1.);
const P1_4: Pos = (1. / 4., 0.);
const P3_4: Pos = (3. / 4., 0.);
const P2_6: Pos = (2. / 6., 0.);
const P3_6: Pos = (3. / 6., 0.);
const P5_6: Pos = (5. / 6., 0.);
const P1_8: Pos = (1. / 8., 0.);
const P3_8: Pos = (3. / 8., 0.);
const P5_8: Pos = (5. / 8., 0.);
const P7_8: Pos = (7. / 8., 0.);
const P2_9: Pos = (2. / 9., 0.);
const P3_9: Pos = (3. / 9., 0.);
const P5_9: Pos = (5. / 9., 0.);
const P6_9: Pos = (6. / 9., 0.);
const P8_9: Pos = (8. / 9., 0.);
const P2_12: Pos = (2. / 12., 0.);
const P3_12: Pos = (3. / 12., 0.);
const P5_12: Pos = (5. / 12., 0.);
const P6_12: Pos = (6. / 12., 0.);
const P8_12: Pos = (8. / 12., 0.);
const P9_12: Pos = (9. / 12., 0.);
const P11_12: Pos = (11. / 12., 0.);

type Op = (Shape, Pos, Pos, Pos, Pos);

#[rustfmt::skip]
const GLYPHS: [&[Op]; COUNT] = [
    // U+2500 ─
    &[(Light, P0_1, P1_2, P1_1, P1_2)],
    // U+2501 ━
    &[(Heavy, P0_1, P1_2, P1_1, P1_2)],
    // U+2502 │
    &[(Light, P1_2, P0_1, P1_2, P1_1)],
    // U+2503 ┃
    &[(Heavy, P1_2, P0_1, P1_2, P1_1)],
    // U+2504 ┄
    &[(Light, P0_1, P1_2, P2_9, P1_2), (Light, P3_9, P1_2, P5_9, P1_2), (Light, P6_9, P1_2, P8_9, P1_2)],
    // U+2505 ┅
    &[(Heavy, P0_1, P1_2, P2_9, P1_2), (Heavy, P3_9, P1_2, P5_9, P1_2), (Heavy, P6_9, P1_2, P8_9, P1_2)],
    // U+2506 ┆
    &[(Light, P1_2, P0_1, P1_2, P2_9), (Light, P1_2, P3_9, P1_2, P5_9), (Light, P1_2, P6_9, P1_2, P8_9)],
    // U+2507 ┇
    &[(Heavy, P1_2, P0_1, P1_2, P2_9), (Heavy, P1_2, P3_9, P1_2, P5_9), (Heavy, P1_2, P6_9, P1_2, P8_9)],
    // U+2508 ┈
    &[(Light, P0_1, P1_2, P2_12, P1_2), (Light, P3_12, P1_2, P5_12, P1_2), (Light, P6_12, P1_2, P8_12, P1_2), (Light, P9_12, P1_2, P11_12, P1_2)],
    // U+2509 ┉
    &[(Heavy, P0_1, P1_2, P2_12, P1_2), (Heavy, P3_12, P1_2, P5_12, P1_2), (Heavy, P6_12, P1_2, P8_12, P1_2), (Heavy, P9_12, P1_2, P11_12, P1_2)],
    // U+250A ┊
    &[(Light, P1_2, P0_1, P1_2, P2_12), (Light, P1_2, P3_12, P1_2, P5_12), (Light, P1_2, P6_12, P1_2, P8_12), (Light, P1_2, P9_12, P1_2, P11_12)],
    // U+250B ┋
    &[(Heavy, P1_2, P0_1, P1_2, P2_12), (Heavy, P1_2, P3_12, P1_2, P5_12), (Heavy, P1_2, P6_12, P1_2, P8_12), (Heavy, P1_2, P9_12, P1_2, P11_12)],
    // U+250C ┌
    &[(Light, P1_2_SUB_0_5, P1_2, P1_1, P1_2), (Light, P1_2, P1_2, P1_2, P1_1)],
    // U+250D ┍
    &[(Heavy, P1_2_SUB_0_5, P1_2, P1_1, P1_2), (Light, P1_2, P1_2, P1_2, P1_1)],
    // U+250E ┎
    &[(Light, P1_2_SUB_1, P1_2, P1_1, P1_2), (Heavy, P1_2, P1_2, P1_2, P1_1)],
    // U+250F ┏
    &[(Heavy, P1_2_SUB_1, P1_2, P1_1, P1_2), (Heavy, P1_2, P1_2, P1_2, P1_1)],
    // U+2510 ┐
    &[(Light, P0_1, P1_2, P1_2_ADD_0_5, P1_2), (Light, P1_2, P1_2, P1_2, P1_1)],
    // U+2511 ┑
    &[(Heavy, P0_1, P1_2, P1_2_ADD_0_5, P1_2), (Light, P1_2, P1_2, P1_2, P1_1)],
    // U+2512 ┒
    &[(Light, P0_1, P1_2, P1_2_ADD_1, P1_2), (Heavy, P1_2, P1_2, P1_2, P1_1)],
    // U+2513 ┓
    &[(Heavy, P0_1, P1_2, P1_2_ADD_1, P1_2), (Heavy, P1_2, P1_2, P1_2, P1_1)],
    // U+2514 └
    &[(Light, P1_2_SUB_0_5, P1_2, P1_1, P1_2), (Light, P1_2, P0_1, P1_2, P1_2)],
    // U+2515 ┕
    &[(Heavy, P1_2_SUB_0_5, P1_2, P1_1, P1_2), (Light, P1_2, P0_1, P1_2, P1_2)],
    // U+2516 ┖
    &[(Light, P1_2_SUB_1, P1_2, P1_1, P1_2), (Heavy, P1_2, P0_1, P1_2, P1_2)],
    // U+2517 ┗
    &[(Heavy, P1_2_SUB_1, P1_2, P1_1, P1_2), (Heavy, P1_2, P0_1, P1_2, P1_2)],
    // U+2518 ┘
    &[(Light, P0_1, P1_2, P1_2_ADD_0_5, P1_2), (Light, P1_2, P0_1, P1_2, P1_2)],
    // U+2519 ┙
    &[(Heavy, P0_1, P1_2, P1_2_ADD_0_5, P1_2), (Light, P1_2, P0_1, P1_2, P1_2)],
    // U+251A ┚
    &[(Light, P0_1, P1_2, P1_2_ADD_1, P1_2), (Heavy, P1_2, P0_1, P1_2, P1_2)],
    // U+251B ┛
    &[(Heavy, P0_1, P1_2, P1_2_ADD_1, P1_2), (Heavy, P1_2, P0_1, P1_2, P1_2)],
    // U+251C ├
    &[(Light, P1_2, P1_2, P1_1, P1_2), (Light, P1_2, P0_1, P1_2, P1_1)],
    // U+251D ┝
    &[(Heavy, P1_2, P1_2, P1_1, P1_2), (Light, P1_2, P0_1, P1_2, P1_1)],
    // U+251E ┞
    &[(Light, P1_2_SUB_1, P1_2, P1_1, P1_2), (Heavy, P1_2, P0_1, P1_2, P1_2), (Light, P1_2, P1_2, P1_2, P1_1)],
    // U+251F ┟
    &[(Light, P1_2_SUB_1, P1_2, P1_1, P1_2), (Light, P1_2, P0_1, P1_2, P1_2), (Heavy, P1_2, P1_2, P1_2, P1_1)],
    // U+2520 ┠
    &[(Light, P1_2, P1_2, P1_1, P1_2), (Heavy, P1_2, P0_1, P1_2, P1_1)],
    // U+2521 ┡
    &[(Heavy, P1_2_SUB_1, P1_2, P1_1, P1_2), (Heavy, P1_2, P0_1, P1_2, P1_2), (Light, P1_2, P1_2, P1_2, P1_1)],
    // U+2522 ┢
    &[(Heavy, P1_2_SUB_1, P1_2, P1_1, P1_2), (Light, P1_2, P0_1, P1_2, P1_2), (Heavy, P1_2, P1_2, P1_2, P1_1)],
    // U+2523 ┣
    &[(Heavy, P1_2, P1_2, P1_1, P1_2), (Heavy, P1_2, P0_1, P1_2, P1_1)],
    // U+2524 ┤
    &[(Light, P0_1, P1_2, P1_2, P1_2), (Light, P1_2, P0_1, P1_2, P1_1)],
    // U+2525 ┥
    &[(Heavy, P0_1, P1_2, P1_2, P1_2), (Light, P1_2, P0_1, P1_2, P1_1)],
    // U+2526 ┦
    &[(Light, P0_1, P1_2, P1_2_ADD_1, P1_2), (Heavy, P1_2, P0_1, P1_2, P1_2), (Light, P1_2, P1_2, P1_2, P1_1)],
    // U+2527 ┧
    &[(Light, P0_1, P1_2, P1_2_ADD_1, P1_2), (Light, P1_2, P0_1, P1_2, P1_2), (Heavy, P1_2, P1_2, P1_2, P1_1)],
    // U+2528 ┨
    &[(Light, P0_1, P1_2, P1_2, P1_2), (Heavy, P1_2, P0_1, P1_2, P1_1)],
    // U+2529 ┩
    &[(Heavy, P0_1, P1_2, P1_2_ADD_1, P1_2), (Heavy, P1_2, P0_1, P1_2, P1_2), (Light, P1_2, P1_2, P1_2, P1_1)],
    // U+252A ┪
    &[(Heavy, P0_1, P1_2, P1_2_ADD_1, P1_2), (Light, P1_2, P0_1, P1_2, P1_2), (Heavy, P1_2, P1_2, P1_2, P1_1)],
    // U+252B ┫
    &[(Heavy, P0_1, P1_2, P1_2, P1_2), (Heavy, P1_2, P0_1, P1_2, P1_1)],
    // U+252C ┬
    &[(Light, P0_1, P1_2, P1_1, P1_2), (Light, P1_2, P1_2, P1_2, P1_1)],
    // U+252D ┭
    &[(Heavy, P0_1, P1_2, P1_2_ADD_0_5, P1_2), (Light, P1_2, P1_2, P1_1, P1_2), (Light, P1_2, P1_2, P1_2, P1_1)],
    // U+252E ┮
    &[(Light, P0_1, P1_2, P1_2, P1_2), (Heavy, P1_2_SUB_0_5, P1_2, P1_1, P1_2), (Light, P1_2, P1_2, P1_2, P1_1)],
    // U+252F ┯
    &[(Heavy, P0_1, P1_2, P1_1, P1_2), (Light, P1_2, P1_2, P1_2, P1_1)],
    // U+2530 ┰
    &[(Light, P0_1, P1_2, P1_1, P1_2), (Heavy, P1_2, P1_2, P1_2, P1_1)],
    // U+2531 ┱
    &[(Heavy, P0_1, P1_2, P1_2_ADD_1, P1_2), (Light, P1_2, P1_2, P1_1, P1_2), (Heavy, P1_2, P1_2, P1_2, P1_1)],
    // U+2532 ┲
    &[(Light, P0_1, P1_2, P1_2, P1_2), (Heavy, P1_2_SUB_1, P1_2, P1_1, P1_2), (Heavy, P1_2, P1_2, P1_2, P1_1)],
    // U+2533 ┳
    &[(Heavy, P0_1, P1_2, P1_1, P1_2), (Heavy, P1_2, P1_2, P1_2, P1_1)],
    // U+2534 ┴
    &[(Light, P0_1, P1_2, P1_1, P1_2), (Light, P1_2, P0_1, P1_2, P1_2)],
    // U+2535 ┵
    &[(Heavy, P0_1, P1_2, P1_2_ADD_0_5, P1_2), (Light, P1_2, P1_2, P1_1, P1_2), (Light, P1_2, P0_1, P1_2, P1_2)],
    // U+2536 ┶
    &[(Light, P0_1, P1_2, P1_2, P1_2), (Heavy, P1_2_SUB_0_5, P1_2, P1_1, P1_2), (Light, P1_2, P0_1, P1_2, P1_2)],
    // U+2537 ┷
    &[(Heavy, P0_1, P1_2, P1_1, P1_2), (Light, P1_2, P0_1, P1_2, P1_2)],
    // U+2538 ┸
    &[(Light, P0_1, P1_2, P1_1, P1_2), (Heavy, P1_2, P0_1, P1_2, P1_2)],
    // U+2539 ┹
    &[(Heavy, P0_1, P1_2, P1_2_ADD_1, P1_2), (Light, P1_2, P1_2, P1_1, P1_2), (Heavy, P1_2, P0_1, P1_2, P1_2)],
    // U+253A ┺
    &[(Light, P0_1, P1_2, P1_2, P1_2), (Heavy, P1_2_SUB_1, P1_2, P1_1, P1_2), (Heavy, P1_2, P0_1, P1_2, P1_2)],
    // U+253B ┻
    &[(Heavy, P0_1, P1_2, P1_1, P1_2), (Heavy, P1_2, P0_1, P1_2, P1_2)],
    // U+253C ┼
    &[(Light, P0_1, P1_2, P1_1, P1_2), (Light, P1_2, P0_1, P1_2, P1_1)],
    // U+253D ┽
    &[(Heavy, P0_1, P1_2, P1_2, P1_2), (Light, P1_2, P1_2, P1_1, P1_2), (Light, P1_2, P0_1, P1_2, P1_1)],
    // U+253E ┾
    &[(Light, P0_1, P1_2, P1_2, P1_2), (Heavy, P1_2, P1_2, P1_1, P1_2), (Light, P1_2, P0_1, P1_2, P1_1)],
    // U+253F ┿
    &[(Heavy, P0_1, P1_2, P1_1, P1_2), (Light, P1_2, P0_1, P1_2, P1_1)],
    // U+2540 ╀
    &[(Light, P0_1, P1_2, P1_1, P1_2), (Heavy, P1_2, P0_1, P1_2, P1_2), (Light, P1_2, P1_2, P1_2, P1_1)],
    // U+2541 ╁
    &[(Light, P0_1, P1_2, P1_1, P1_2), (Light, P1_2, P0_1, P1_2, P1_2), (Heavy, P1_2, P1_2, P1_2, P1_1)],
    // U+2542 ╂
    &[(Light, P0_1, P1_2, P1_1, P1_2), (Heavy, P1_2, P0_1, P1_2, P1_1)],
    // U+2543 ╃
    &[(Heavy, P0_1, P1_2, P1_2_ADD_1, P1_2), (Light, P1_2, P1_2, P1_1, P1_2), (Heavy, P1_2, P0_1, P1_2, P1_2), (Light, P1_2, P1_2, P1_2, P1_1)],
    // U+2544 ╄
    &[(Light, P0_1, P1_2, P1_2, P1_2), (Heavy, P1_2_SUB_1, P1_2, P1_1, P1_2), (Heavy, P1_2, P0_1, P1_2, P1_2), (Light, P1_2, P1_2, P1_2, P1_1)],
    // U+2545 ╅
    &[(Heavy, P0_1, P1_2, P1_2_ADD_1, P1_2), (Light, P1_2, P1_2, P1_1, P1_2), (Light, P1_2, P0_1, P1_2, P1_2), (Heavy, P1_2, P1_2, P1_2, P1_1)],
    // U+2546 ╆
    &[(Light, P0_1, P1_2, P1_2, P1_2), (Heavy, P1_2_SUB_1, P1_2, P1_1, P1_2), (Light, P1_2, P0_1, P1_2, P1_2), (Heavy, P1_2, P1_2, P1_2, P1_1)],
    // U+2547 ╇
    &[(Heavy, P0_1, P1_2, P1_1, P1_2), (Heavy, P1_2, P0_1, P1_2, P1_2), (Light, P1_2, P1_2, P1_2, P1_1)],
    // U+2548 ╈
    &[(Heavy, P0_1, P1_2, P1_1, P1_2), (Light, P1_2, P0_1, P1_2, P1_2), (Heavy, P1_2, P1_2, P1_2, P1_1)],
    // U+2549 ╉
    &[(Heavy, P0_1, P1_2, P1_2, P1_2), (Light, P1_2, P1_2, P1_1, P1_2), (Heavy, P1_2, P0_1, P1_2, P1_1)],
    // U+254A ╊
    &[(Light, P0_1, P1_2, P1_2, P1_2), (Heavy, P1_2, P1_2, P1_1, P1_2), (Heavy, P1_2, P0_1, P1_2, P1_1)],
    // U+254B ╋
    &[(Heavy, P0_1, P1_2, P1_1, P1_2), (Heavy, P1_2, P0_1, P1_2, P1_1)],
    // U+254C ╌
    &[(Light, P0_1, P1_2, P2_6, P1_2), (Light, P3_6, P1_2, P5_6, P1_2)],
    // U+254D ╍
    &[(Heavy, P0_1, P1_2, P2_6, P1_2), (Heavy, P3_6, P1_2, P5_6, P1_2)],
    // U+254E ╎
    &[(Light, P1_2, P0_1, P1_2, P2_6), (Light, P1_2, P3_6, P1_2, P5_6)],
    // U+254F ╏
    &[(Heavy, P1_2, P0_1, P1_2, P2_6), (Heavy, P1_2, P3_6, P1_2, P5_6)],
    // U+2550 ═
    &[(Light, P0_1, P1_2_SUB_1, P1_1, P1_2_SUB_1), (Light, P0_1, P1_2_ADD_1, P1_1, P1_2_ADD_1)],
    // U+2551 ║
    &[(Light, P1_2_SUB_1, P0_1, P1_2_SUB_1, P1_1), (Light, P1_2_ADD_1, P0_1, P1_2_ADD_1, P1_1)],
    // U+2552 ╒
    &[(Light, P1_2_SUB_0_5, P1_2_SUB_1, P1_1, P1_2_SUB_1), (Light, P1_2_SUB_0_5, P1_2_ADD_1, P1_1, P1_2_ADD_1), (Light, P1_2, P1_2_SUB_1, P1_2, P1_1)],
    // U+2553 ╓
    &[(Light, P1_2_SUB_1, P1_2_SUB_0_5, P1_2_SUB_1, P1_1), (Light, P1_2_ADD_1, P1_2_SUB_0_5, P1_2_ADD_1, P1_1), (Light, P1_2_SUB_1, P1_2, P1_1, P1_2)],
    // U+2554 ╔
    &[(Rect, P1_2_SUB_1, P1_2_SUB_1, MAX, MAX), (Rect, P1_2_ADD_1, P1_2_ADD_1, MAX, MAX)],
    // U+2555 ╕
    &[(Light, P0_1, P1_2_SUB_1, P1_2_ADD_0_5, P1_2_SUB_1), (Light, P0_1, P1_2_ADD_1, P1_2_ADD_0_5, P1_2_ADD_1), (Light, P1_2, P1_2_SUB_1, P1_2, P1_1)],
    // U+2556 ╖
    &[(Light, P1_2_SUB_1, P1_2_SUB_0_5, P1_2_SUB_1, P1_1), (Light, P1_2_ADD_1, P1_2_SUB_0_5, P1_2_ADD_1, P1_1), (Light, P0_1, P1_2, P1_2_ADD_1, P1_2)],
    // U+2557 ╗
    &[(Rect, MIN, P1_2_SUB_1, P1_2_ADD_1, MAX), (Rect, MIN, P1_2_ADD_1, P1_2_SUB_1, MAX)],
    // U+2558 ╘
    &[(Light, P1_2_SUB_0_5, P1_2_SUB_1, P1_1, P1_2_SUB_1), (Light, P1_2_SUB_0_5, P1_2_ADD_1, P1_1, P1_2_ADD_1), (Light, P1_2, P0_1, P1_2, P1_2_ADD_1)],
    // U+2559 ╙
    &[(Light, P1_2_SUB_1, P0_1, P1_2_SUB_1, P1_2_ADD_0_5), (Light, P1_2_ADD_1, P0_1, P1_2_ADD_1, P1_2_ADD_0_5), (Light, P1_2_SUB_1, P1_2, P1_1, P1_2)],
    // U+255A ╚
    &[(Rect, P1_2_SUB_1, MIN, MAX, P1_2_ADD_1), (Rect, P1_2_ADD_1, MIN, MAX, P1_2_SUB_1)],
    // U+255B ╛
    &[(Light, P0_1, P1_2_SUB_1, P1_2_ADD_0_5, P1_2_SUB_1), (Light, P0_1, P1_2_ADD_1, P1_2_ADD_0_5, P1_2_ADD_1), (Light, P1_2, P0_1, P1_2, P1_2_ADD_1)],
    // U+255C ╜
    &[(Light, P1_2_SUB_1, P0_1, P1_2_SUB_1, P1_2_ADD_0_5), (Light, P1_2_ADD_1, P0_1, P1_2_ADD_1, P1_2_ADD_0_5), (Light, P0_1, P1_2, P1_2_ADD_1, P1_2)],
    // U+255D ╝
    &[(Rect, MIN, MIN, P1_2_ADD_1, P1_2_ADD_1), (Rect, MIN, MIN, P1_2_SUB_1, P1_2_SUB_1)],
    // U+255E ╞
    &[(Light, P1_2, P1_2_SUB_1, P1_1, P1_2_SUB_1), (Light, P1_2, P1_2_ADD_1, P1_1, P1_2_ADD_1), (Light, P1_2, P0_1, P1_2, P1_1)],
    // U+255F ╟
    &[(Light, P1_2_SUB_1, P0_1, P1_2_SUB_1, P1_1), (Light, P1_2_ADD_1, P0_1, P1_2_ADD_1, P1_1), (Light, P1_2_ADD_1, P1_2, P1_1, P1_2)],
    // U+2560 ╠
    &[(Light, P1_2_SUB_1, P0_1, P1_2_SUB_1, P1_1), (Rect, P1_2_ADD_1, MIN, MAX, P1_2_SUB_1), (Rect, P1_2_ADD_1, P1_2_ADD_1, MAX, MAX)],
    // U+2561 ╡
    &[(Light, P0_1, P1_2_SUB_1, P1_2, P1_2_SUB_1), (Light, P0_1, P1_2_ADD_1, P1_2, P1_2_ADD_1), (Light, P1_2, P0_1, P1_2, P1_1)],
    // U+2562 ╢
    &[(Light, P1_2_SUB_1, P0_1, P1_2_SUB_1, P1_1), (Light, P1_2_ADD_1, P0_1, P1_2_ADD_1, P1_1), (Light, P0_1, P1_2, P1_2_SUB_1, P1_2)],
    // U+2563 ╣
    &[(Light, P1_2_ADD_1, P0_1, P1_2_ADD_1, P1_1), (Rect, MIN, MIN, P1_2_SUB_1, P1_2_SUB_1), (Rect, MIN, P1_2_ADD_1, P1_2_SUB_1, MAX)],
    // U+2564 ╤
    &[(Light, P0_1, P1_2_SUB_1, P1_1, P1_2_SUB_1), (Light, P0_1, P1_2_ADD_1, P1_1, P1_2_ADD_1), (Light, P1_2, P1_2_ADD_1, P1_2, P1_1)],
    // U+2565 ╥
    &[(Light, P0_1, P1_2, P1_1, P1_2), (Light, P1_2_SUB_1, P1_2, P1_2_SUB_1, P1_1), (Light, P1_2_ADD_1, P1_2, P1_2_ADD_1, P1_1)],
    // U+2566 ╦
    &[(Light, P0_1, P1_2_SUB_1, P1_1, P1_2_SUB_1), (Rect, MIN, P1_2_ADD_1, P1_2_SUB_1, MAX), (Rect, P1_2_ADD_1, P1_2_ADD_1, MAX, MAX)],
    // U+2567 ╧
    &[(Light, P0_1, P1_2_SUB_1, P1_1, P1_2_SUB_1), (Light, P0_1, P1_2_ADD_1, P1_1, P1_2_ADD_1), (Light, P1_2, P0_1, P1_2, P1_2_SUB_1)],
    // U+2568 ╨
    &[(Light, P0_1, P1_2, P1_1, P1_2), (Light, P1_2_SUB_1, P0_1, P1_2_SUB_1, P1_2), (Light, P1_2_ADD_1, P0_1, P1_2_ADD_1, P1_2)],
    // U+2569 ╩
    &[(Light, P0_1, P1_2_ADD_1, P1_1, P1_2_ADD_1), (Rect, MIN, MIN, P1_2_SUB_1, P1_2_SUB_1), (Rect, P1_2_ADD_1, MIN, MAX, P1_2_SUB_1)],
    // U+256A ╪
    &[(Light, P0_1, P1_2_SUB_1, P1_1, P1_2_SUB_1), (Light, P0_1, P1_2_ADD_1, P1_1, P1_2_ADD_1), (Light, P1_2, P0_1, P1_2, P1_1)],
    // U+256B ╫
    &[(Light, P1_2_SUB_1, P0_1, P1_2_SUB_1, P1_1), (Light, P1_2_ADD_1, P0_1, P1_2_ADD_1, P1_1), (Light, P0_1, P1_2, P1_1, P1_2)],
    // U+256C ╬
    &[(Rect, MIN, MIN, P1_2_SUB_1, P1_2_SUB_1), (Rect, P1_2_ADD_1, MIN, MAX, P1_2_SUB_1), (Rect, MIN, P1_2_ADD_1, P1_2_SUB_1, MAX), (Rect, P1_2_ADD_1, P1_2_ADD_1, MAX, MAX)],
    // U+256D ╭
    &[(Round, P1_2, P1_2, MAX, MAX)],
    // U+256E ╮
    &[(Round, MIN, P1_2, P1_2, MAX)],
    // U+256F ╯
    &[(Round, MIN, MIN, P1_2, P1_2)],
    // U+2570 ╰
    &[(Round, P1_2, MIN, MAX, P1_2)],
    // U+2571 ╱
    &[(Light, P0_1, P1_1, P1_1, P0_1)],
    // U+2572 ╲
    &[(Light, P0_1, P0_1, P1_1, P1_1)],
    // U+2573 ╳
    &[(Light, P0_1, P1_1, P1_1, P0_1), (Light, P0_1, P0_1, P1_1, P1_1)],
    // U+2574 ╴
    &[(Light, P0_1, P1_2, P1_2, P1_2)],
    // U+2575 ╵
    &[(Light, P1_2, P0_1, P1_2, P1_2)],
    // U+2576 ╶
    &[(Light, P1_2, P1_2, P1_1, P1_2)],
    // U+2577 ╷
    &[(Light, P1_2, P1_2, P1_2, P1_1)],
    // U+2578 ╸
    &[(Heavy, P0_1, P1_2, P1_2, P1_2)],
    // U+2579 ╹
    &[(Heavy, P1_2, P0_1, P1_2, P1_2)],
    // U+257A ╺
    &[(Heavy, P1_2, P1_2, P1_1, P1_2)],
    // U+257B ╻
    &[(Heavy, P1_2, P1_2, P1_2, P1_1)],
    // U+257C ╼
    &[(Light, P0_1, P1_2, P1_2, P1_2), (Heavy, P1_2, P1_2, P1_1, P1_2)],
    // U+257D ╽
    &[(Light, P1_2, P0_1, P1_2, P1_2), (Heavy, P1_2, P1_2, P1_2, P1_1)],
    // U+257E ╾
    &[(Heavy, P0_1, P1_2, P1_2, P1_2), (Light, P1_2, P1_2, P1_1, P1_2)],
    // U+257F ╿
    &[(Heavy, P1_2, P0_1, P1_2, P1_2), (Light, P1_2, P1_2, P1_2, P1_1)],
    // U+2580 ▀
    &[(Fill, P0_1, P0_1, P1_1, P1_2)],
    // U+2581 ▁
    &[(Fill, P0_1, P7_8, P1_1, P1_1)],
    // U+2582 ▂
    &[(Fill, P0_1, P3_4, P1_1, P1_1)],
    // U+2583 ▃
    &[(Fill, P0_1, P5_8, P1_1, P1_1)],
    // U+2584 ▄
    &[(Fill, P0_1, P1_2, P1_1, P1_1)],
    // U+2585 ▅
    &[(Fill, P0_1, P3_8, P1_1, P1_1)],
    // U+2586 ▆
    &[(Fill, P0_1, P1_4, P1_1, P1_1)],
    // U+2587 ▇
    &[(Fill, P0_1, P1_8, P1_1, P1_1)],
    // U+2588 █
    &[(Fill, P0_1, P0_1, P1_1, P1_1)],
    // U+2589 ▉
    &[(Fill, P0_1, P0_1, P7_8, P1_1)],
    // U+258A ▊
    &[(Fill, P0_1, P0_1, P3_4, P1_1)],
    // U+258B ▋
    &[(Fill, P0_1, P0_1, P5_8, P1_1)],
    // U+258C ▌
    &[(Fill, P0_1, P0_1, P1_2, P1_1)],
    // U+258D ▍
    &[(Fill, P0_1, P0_1, P3_8, P1_1)],
    // U+258E ▎
    &[(Fill, P0_1, P0_1, P1_4, P1_1)],
    // U+258F ▏
    &[(Fill, P0_1, P0_1, P1_8, P1_1)],
    // U+2590 ▐
    &[(Fill, P1_2, P0_1, P1_1, P1_1)],
    // U+2591 ░
    &[(Shade(0.25), P0_1, P0_1, P1_1, P1_1)],
    // U+2592 ▒
    &[(Shade(0.5), P0_1, P0_1, P1_1, P1_1)],
    // U+2593 ▓
    &[(Shade(0.75), P0_1, P0_1, P1_1, P1_1)],
    // U+2594 ▔
    &[(Fill, P0_1, P0_1, P1_1, P1_8)],
    // U+2595 ▕
    &[(Fill, P7_8, P0_1, P1_1, P1_1)],
    // U+2596 ▖
    &[(Fill, P0_1, P1_2, P1_2, P1_1)],
    // U+2597 ▗
    &[(Fill, P1_2, P1_2, P1_1, P1_1)],
    // U+2598 ▘
    &[(Fill, P0_1, P0_1, P1_2, P1_2)],
    // U+2599 ▙
    &[(Fill, P0_1, P0_1, P1_2, P1_1), (Fill, P1_2, P1_2, P1_1, P1_1)],
    // U+259A ▚
    &[(Fill, P0_1, P0_1, P1_2, P1_2), (Fill, P1_2, P1_2, P1_1, P1_1)],
    // U+259B ▛
    &[(Fill, P0_1, P0_1, P1_2, P1_1), (Fill, P1_2, P0_1, P1_1, P1_2)],
    // U+259C ▜
    &[(Fill, P0_1, P0_1, P1_2, P1_2), (Fill, P1_2, P0_1, P1_1, P1_1)],
    // U+259D ▝
    &[(Fill, P1_2, P0_1, P1_1, P1_2)],
    // U+259E ▞
    &[(Fill, P0_1, P1_2, P1_2, P1_1), (Fill, P1_2, P0_1, P1_1, P1_2)],
    // U+259F ▟
    &[(Fill, P0_1, P1_2, P1_2, P1_1), (Fill, P1_2, P0_1, P1_1, P1_1)],
];

/// A rectangle `[left, top, right, bottom]` in device pixels from the cell's top-left corner.
pub(crate) type Quad = [f32; 4];

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Mark {
    /// Disjoint from the glyph's other quads, so a translucent colour does not darken where strokes meet.
    Quads(Vec<Quad>, f32),
    /// A stroke of this width through the points, closed or open; the painter clips it to the cell.
    Stroke(Vec<[f32; 2]>, f32),
    /// A rounded rectangle's outline `rect`, corner radius and stroke width; clipped likewise.
    RoundRect(Quad, f32, f32),
}

/// Windows Terminal's light line: a sixth of the cell width, at least one pixel (DrawBuiltinGlyph).
pub(crate) fn light_line(cell_width: f32) -> f32 {
    (cell_width / 6.).round().max(1.)
}

/// What `ch` paints in a cell `w`×`h` device pixels whose edges are on whole pixels.
pub(crate) fn marks(ch: char, w: f32, h: f32, light: f32) -> Vec<Mark> {
    let Some(ops) = (ch as u32).checked_sub(FIRST).and_then(|i| GLYPHS.get(i as usize)) else {
        return Vec::new();
    };
    let radius = (light * 5.).min(w.min(h) * 0.5);
    let mut quads = Vec::new();
    let mut opacity = 1.;
    let mut marks = Vec::new();
    for &(shape, beg_x, beg_y, end_x, end_y) in *ops {
        let at = |p: Pos, size: f32| p.0 * size + p.1 * light;
        let (bx, by, ex, ey) = (at(beg_x, w), at(beg_y, h), at(end_x, w), at(end_y, h));
        let width = if shape == Heavy { light * 2. } else { light };
        let half = width / 2.;
        let hollow = matches!(shape, Rect | Round);
        let line = matches!(shape, Light | Heavy);
        let (line_x, line_y) = (line && bx == ex, line && by == ey);
        let off_x = if hollow || line_x { half } else { 0. };
        let off_y = if hollow || line_y { half } else { 0. };
        let (bx, by) = ((bx - off_x).round() + off_x, (by - off_y).round() + off_y);
        let (ex, ey) = ((ex + off_x).round() - off_x, (ey + off_y).round() - off_y);
        match shape {
            Shade(alpha) => {
                opacity = alpha;
                quads.push([bx, by, ex, ey]);
            }
            Fill => quads.push([bx, by, ex, ey]),
            Light | Heavy if line_y => quads.push([bx, by - half, ex, by + half]),
            Light | Heavy if line_x => quads.push([bx - half, by, bx + half, ey]),
            Light | Heavy => marks.push(Mark::Stroke(vec![[bx, by], [ex, ey]], width)),
            Rect => quads.extend([
                [bx - half, by - half, ex + half, by + half],
                [bx - half, ey - half, ex + half, ey + half],
                [bx - half, by - half, bx + half, ey + half],
                [ex - half, by - half, ex + half, ey + half],
            ]),
            Round => marks.push(Mark::RoundRect([bx, by, ex, ey], radius, width)),
        }
    }
    let quads = disjoint(quads.into_iter().filter_map(|q| clip(q, w, h)).collect());
    if !quads.is_empty() {
        marks.insert(0, Mark::Quads(quads, opacity));
    }
    marks
}

fn clip([l, t, r, b]: Quad, w: f32, h: f32) -> Option<Quad> {
    let q = [l.max(0.), t.max(0.), r.min(w), b.min(h)];
    (q[0] < q[2] && q[1] < q[3]).then_some(q)
}

/// The union of `quads` as horizontal bands of disjoint spans.
fn disjoint(quads: Vec<Quad>) -> Vec<Quad> {
    if quads.len() < 2 {
        return quads;
    }
    let mut ys: Vec<f32> = quads.iter().flat_map(|q| [q[1], q[3]]).collect();
    ys.sort_by(f32::total_cmp);
    ys.dedup();
    let mut out: Vec<Quad> = Vec::new();
    for band in ys.windows(2) {
        let (top, bottom) = (band[0], band[1]);
        let mut spans: Vec<(f32, f32)> = quads.iter().filter(|q| q[1] <= top && q[3] >= bottom).map(|q| (q[0], q[2])).collect();
        spans.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut merged: Vec<(f32, f32)> = Vec::new();
        for (l, r) in spans {
            match merged.last_mut() {
                Some(last) if l <= last.1 => last.1 = last.1.max(r),
                _ => merged.push((l, r)),
            }
        }
        for (l, r) in merged {
            match out.iter_mut().find(|q| q[0] == l && q[2] == r && q[3] == top) {
                Some(q) => q[3] = bottom,
                None => out.push([l, top, r, bottom]),
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quads(ch: char, w: f32, h: f32) -> Vec<Quad> {
        match marks(ch, w, h, light_line(w)).first() {
            Some(Mark::Quads(q, _)) => q.clone(),
            other => panic!("`{ch}` paints {other:?}"),
        }
    }

    fn inked(quads: &[Quad], x: f32, y: f32) -> bool {
        quads.iter().any(|q| q[0] <= x && x < q[2] && q[1] <= y && y < q[3])
    }

    #[test]
    fn the_table_covers_box_drawing_and_block_elements_and_nothing_else() {
        assert!(is_builtin('─') && is_builtin('╳') && is_builtin('▟') && is_builtin('░'));
        assert!(!is_builtin('\u{24FF}') && !is_builtin('\u{25A0}') && !is_builtin('a'));
        for i in 0..COUNT as u32 {
            let ch = char::from_u32(FIRST + i).unwrap();
            assert!(!marks(ch, 9., 18., 2.).is_empty(), "U+{:04X} paints nothing", FIRST + i);
        }
    }

    #[test]
    fn every_quad_is_inside_its_cell_on_whole_pixels() {
        for (w, h) in [(8., 18.), (9., 23.), (13., 31.), (3., 6.)] {
            for i in 0..COUNT as u32 {
                let ch = char::from_u32(FIRST + i).unwrap();
                for mark in marks(ch, w, h, light_line(w)) {
                    if let Mark::Quads(quads, _) = mark {
                        for q in quads {
                            assert!(q[0] >= 0. && q[1] >= 0. && q[2] <= w && q[3] <= h, "U+{:04X} in {w}×{h}: {q:?}", FIRST + i);
                            assert!(q.iter().all(|v| v.fract() == 0.), "U+{:04X} in {w}×{h} is off the pixel grid: {q:?}", FIRST + i);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn horizontals_span_the_cell_and_verticals_its_height_so_neighbours_join() {
        for (w, h) in [(8., 18.), (9., 18.), (11., 26.)] {
            let lw = light_line(w);
            let row = |ch| quads(ch, w, h);
            let horizontal = row('─');
            assert_eq!(horizontal.len(), 1);
            assert_eq!((horizontal[0][0], horizontal[0][2], horizontal[0][3] - horizontal[0][1]), (0., w, lw));
            let vertical = row('│');
            assert_eq!((vertical[0][1], vertical[0][3], vertical[0][2] - vertical[0][0]), (0., h, lw));
            for ch in ['┌', '┬', '┐', '├', '┼', '┤', '└', '┴', '┘'] {
                let q = row(ch);
                let (cx, cy) = (vertical[0][0], horizontal[0][1]);
                assert!(inked(&q, cx, cy), "`{ch}` in {w}×{h} has no ink where its strokes meet");
                for (x, y, reaches) in [(0., cy, "┬┐┼┤┴┘"), (w - 1., cy, "┌┬├┼└┴"), (cx, 0., "├┼┤└┴┘"), (cx, h - 1., "┌┬┐├┼┤")] {
                    assert_eq!(inked(&q, x, y), reaches.contains(ch), "`{ch}` in {w}×{h} at ({x}, {y})");
                }
                for (x, y) in [(cx, cy), (cx + lw - 1., cy + lw - 1.)] {
                    assert!(inked(&q, x, y), "`{ch}` in {w}×{h}: the joint misses ({x}, {y})");
                }
            }
        }
    }

    #[test]
    fn heavy_strokes_are_twice_the_light_ones_and_double_lines_two_light_ones() {
        let (w, h) = (12., 24.);
        let lw = light_line(w);
        assert_eq!(lw, 2.);
        let heavy = quads('━', w, h);
        assert_eq!(heavy[0][3] - heavy[0][1], 2. * lw);
        let double = quads('═', w, h);
        assert_eq!(double.len(), 2);
        assert!(double.iter().all(|q| q[3] - q[1] == lw && q[0] == 0. && q[2] == w));
        assert_eq!(double[1][1] - double[0][3], lw, "one light line between the two");
    }

    #[test]
    fn crossing_strokes_are_disjoint_so_a_translucent_colour_is_even() {
        for ch in ['┼', '╋', '╬', '╔', '▙'] {
            let q = quads(ch, 10., 20.);
            for (i, a) in q.iter().enumerate() {
                for b in &q[i + 1..] {
                    let overlap = a[0].max(b[0]) < a[2].min(b[2]) && a[1].max(b[1]) < a[3].min(b[3]);
                    assert!(!overlap, "`{ch}`: {a:?} overlaps {b:?}");
                }
            }
        }
    }

    #[test]
    fn dashes_arcs_diagonals_and_shades_follow_windows_terminal() {
        assert_eq!(quads('┄', 9., 18.).len(), 3);
        assert_eq!(quads('┈', 12., 18.).len(), 4);
        assert!(matches!(marks('╭', 9., 18., 2.).as_slice(), [Mark::RoundRect(..)]));
        assert!(matches!(marks('╳', 9., 18., 2.).as_slice(), [Mark::Stroke(..), Mark::Stroke(..)]));
        assert_eq!(marks('▒', 9., 18., 2.), [Mark::Quads(vec![[0., 0., 9., 18.]], 0.5)]);
        assert_eq!(quads('▄', 9., 18.), [[0., 9., 9., 18.]]);
        assert_eq!(quads('█', 9., 18.), [[0., 0., 9., 18.]]);
    }
}
