//! Docking and snapping of skinned windows (05 §6). Pure geometry.

use crate::geom::Rect;

/// Default snap distance in pixels.
pub const DEFAULT_SNAP: i32 = 10;

fn close(a: i32, b: i32, dist: i32) -> bool {
    (a - b).abs() <= dist
}

/// Snaps the edges of `r` to the work area edges within `dist` (05 §6.1).
pub fn snap_to_screen(mut r: Rect, work: Rect, dist: i32) -> Rect {
    if close(r.x, work.x, dist) {
        r.x = work.x;
    } else if close(r.right(), work.right(), dist) {
        r.x = work.right() - r.w;
    }
    if close(r.y, work.y, dist) {
        r.y = work.y;
    } else if close(r.bottom(), work.bottom(), dist) {
        r.y = work.bottom() - r.h;
    }
    r
}

/// Snaps a moving window `s` against one other window `d` (rules 1–4 of 05 §6.1).
pub fn snap_to_window(mut s: Rect, d: Rect, dist: i32) -> Rect {
    if s.overlaps_x(&d) {
        if close(s.y, d.bottom(), dist) {
            s.y = d.bottom();
        } else if close(s.bottom(), d.y, dist) {
            s.y = d.y - s.h;
        }
    }
    if s.overlaps_y(&d) {
        if close(s.right(), d.x, dist) {
            s.x = d.x - s.w;
        } else if close(s.x, d.right(), dist) {
            s.x = d.right();
        }
    }
    let side_by_side = (s.right() == d.x || s.x == d.right()) && s.overlaps_y(&d);
    if side_by_side {
        if close(s.y, d.y, dist) {
            s.y = d.y;
        } else if close(s.bottom(), d.bottom(), dist) {
            s.y = d.bottom() - s.h;
        }
    }
    let stacked = (s.y == d.bottom() || s.bottom() == d.y) && s.overlaps_x(&d);
    if stacked {
        if close(s.x, d.x, dist) {
            s.x = d.x;
        } else if close(s.right(), d.right(), dist) {
            s.x = d.right() - s.w;
        }
    }
    s
}

/// Full snap of a moving window: screen first (when `work` is given), then each other window
/// in order.
pub fn snap(s: Rect, others: &[Rect], work: Option<Rect>, dist: i32) -> Rect {
    let mut r = match work {
        Some(w) => snap_to_screen(s, w, dist),
        None => s,
    };
    for d in others {
        r = snap_to_window(r, *d, dist);
    }
    r
}

/// Two windows are attached when an edge of one equals the opposite edge of the other and they
/// overlap along it (05 §6.2). Windows that meet only at a corner are not attached.
pub fn attached(a: &Rect, b: &Rect) -> bool {
    ((a.bottom() == b.y || b.bottom() == a.y) && a.shares_x(b))
        || ((a.right() == b.x || b.right() == a.x) && a.shares_y(b))
}

/// Indices of all windows transitively attached to `root` (not including `root`).
pub fn attached_group(rects: &[Rect], root: usize) -> Vec<usize> {
    let mut group = vec![root];
    let mut i = 0;
    while i < group.len() {
        let cur = rects[group[i]];
        for (j, r) in rects.iter().enumerate() {
            if !group.contains(&j) && attached(&cur, r) {
                group.push(j);
            }
        }
        i += 1;
    }
    group.remove(0);
    group
}

/// Moves a group: the leader moves to `target` (already offset by the pointer delta), is snapped
/// to the screen and to windows outside the group, and every group member follows by the same
/// delta. With `work`, group members are also kept on screen. Returns new rects for all windows.
pub fn move_group(
    rects: &[Rect],
    leader: usize,
    group: &[usize],
    target: Rect,
    work: Option<Rect>,
    dist: i32,
) -> Vec<Rect> {
    let outsiders: Vec<Rect> = rects
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != leader && !group.contains(i))
        .map(|(_, r)| *r)
        .collect();
    let snapped = snap(target, &outsiders, work, dist);
    let (mut dx, mut dy) = (snapped.x - rects[leader].x, snapped.y - rects[leader].y);
    if let Some(w) = work {
        // If a group member would snap to a screen edge, shift the whole group to match.
        let (orig_dx, orig_dy) = (dx, dy);
        for &g in group {
            let moved = rects[g].translated(orig_dx, orig_dy);
            let s = snap_to_screen(moved, w, dist);
            if s.x != moved.x && dx == orig_dx && snapped.x == target.x {
                dx += s.x - moved.x;
            }
            if s.y != moved.y && dy == orig_dy && snapped.y == target.y {
                dy += s.y - moved.y;
            }
        }
    }
    let mut out = rects.to_vec();
    out[leader] = rects[leader].translated(dx, dy);
    for &g in group {
        out[g] = rects[g].translated(dx, dy);
    }
    out
}

/// Applies a size change of window `idx` from `old` to `new` and moves docked windows so they
/// stay attached (05 §6.3). `work` enables the "flush with the screen edge" rule.
pub fn resize_propagate(rects: &mut [Rect], idx: usize, old: Rect, new: Rect, work: Option<Rect>) {
    rects[idx] = new;
    let dy = new.bottom() - old.bottom();
    let dx = new.right() - old.right();
    if dy != 0 {
        shift_below(rects, idx, old, dy);
    }
    if dx != 0 {
        shift_right(rects, idx, old, dx);
    }
    // Windows flush with the bottom of the screen stay flush; windows docked above follow.
    if let Some(w) = work {
        if dy != 0 && old.bottom() == w.bottom() {
            let shift = -dy;
            let above: Vec<usize> = chain_above(rects, idx, old);
            rects[idx] = rects[idx].translated(0, shift);
            for i in above {
                rects[i] = rects[i].translated(0, shift);
            }
        }
        if dx != 0 && old.right() == w.right() {
            let shift = -dx;
            rects[idx] = rects[idx].translated(shift, 0);
        }
    }
}

fn shift_below(rects: &mut [Rect], idx: usize, old: Rect, dy: i32) {
    let movers: Vec<(usize, Rect)> = rects
        .iter()
        .enumerate()
        .filter(|(i, r)| *i != idx && r.y == old.bottom() && r.shares_x(&old))
        .map(|(i, r)| (i, *r))
        .collect();
    for (i, r) in movers {
        if rects[i] == r {
            rects[i] = r.translated(0, dy);
            shift_below(rects, i, r, dy);
        }
    }
}

fn shift_right(rects: &mut [Rect], idx: usize, old: Rect, dx: i32) {
    let movers: Vec<(usize, Rect)> = rects
        .iter()
        .enumerate()
        .filter(|(i, r)| *i != idx && r.x == old.right() && r.shares_y(&old))
        .map(|(i, r)| (i, *r))
        .collect();
    for (i, r) in movers {
        if rects[i] == r {
            rects[i] = r.translated(dx, 0);
            shift_right(rects, i, r, dx);
        }
    }
}

fn chain_above(rects: &[Rect], idx: usize, old: Rect) -> Vec<usize> {
    let mut out = Vec::new();
    let mut frontier = vec![old];
    while let Some(cur) = frontier.pop() {
        for (i, r) in rects.iter().enumerate() {
            if i != idx && !out.contains(&i) && r.bottom() == cur.y && r.overlaps_x(&cur) {
                out.push(i);
                frontier.push(*r);
            }
        }
    }
    out
}

/// Moves a window back on screen when it is more than `margin` pixels off-screen (02 §14).
pub fn keep_on_screen(r: Rect, work: Rect, margin: i32) -> Rect {
    let mut r = r;
    if r.x < work.x - margin {
        r.x = work.x;
    }
    if r.y < work.y - margin {
        r.y = work.y;
    }
    if r.right() > work.right() + margin {
        r.x = work.right() - r.w;
    }
    if r.bottom() > work.bottom() + margin {
        r.y = work.bottom() - r.h;
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: Rect = Rect::new(26, 29, 275, 116);
    const EQ: Rect = Rect::new(26, 145, 275, 116);
    const PL: Rect = Rect::new(26, 261, 275, 116);

    #[test]
    fn default_stack_is_one_group() {
        let rects = [MAIN, EQ, PL];
        assert_eq!(attached_group(&rects, 0), vec![1, 2]);
    }

    #[test]
    fn group_follows_main() {
        let rects = [MAIN, EQ, PL];
        let group = attached_group(&rects, 0);
        let out = move_group(&rects, 0, &group, MAIN.translated(100, 50), None, 10);
        assert_eq!(out[1], EQ.translated(100, 50));
        assert_eq!(out[2], PL.translated(100, 50));
    }

    #[test]
    fn shade_moves_docked_up_102() {
        let mut rects = [MAIN, EQ, PL];
        let shaded = Rect::new(26, 29, 275, 14);
        resize_propagate(&mut rects, 0, MAIN, shaded, None);
        assert_eq!(rects[1].y, 145 - 102);
        assert_eq!(rects[2].y, 261 - 102);
        // Double size moves them down.
        let mut rects = [MAIN, EQ, PL];
        resize_propagate(&mut rects, 0, MAIN, Rect::new(26, 29, 550, 232), None);
        assert_eq!(rects[1].y, 145 + 116);
        assert_eq!(rects[2].y, 261 + 116);
    }

    #[test]
    fn corner_contact_is_not_docking() {
        // Playlist right of main, equalizer below main: they meet at one corner only.
        let pl = Rect::new(301, 29, 275, 116);
        let mut rects = [MAIN, EQ, pl];
        assert!(!attached(&EQ, &pl));
        resize_propagate(&mut rects, 2, pl, Rect::new(301, 29, 275, 232), None);
        assert_eq!(rects[1], EQ);
        assert_eq!(attached_group(&rects, 0), vec![1, 2]);
    }

    #[test]
    fn undocked_window_does_not_follow() {
        let rects = [MAIN, EQ.translated(300, 0), PL];
        // PL is no longer attached to anything but EQ was moved away: PL's top touches EQ's
        // old bottom only.
        let group = attached_group(&rects, 0);
        assert!(!group.contains(&1));
    }

    #[test]
    fn snap_within_distance() {
        let s = Rect::new(26, 155, 275, 116); // 10 px below main
        assert_eq!(snap(s, &[MAIN], None, 10).y, 145);
        let s = Rect::new(26, 156, 275, 116); // 11 px: no snap
        assert_eq!(snap(s, &[MAIN], None, 10).y, 156);
        // Aligns left edges once stacked.
        let s = Rect::new(33, 150, 275, 116);
        assert_eq!(snap(s, &[MAIN], None, 10), Rect::new(26, 145, 275, 116));
        // Side by side, tops aligned.
        let s = Rect::new(305, 35, 275, 116);
        assert_eq!(snap(s, &[MAIN], None, 10), Rect::new(301, 29, 275, 116));
    }

    #[test]
    fn screen_snap() {
        let work = Rect::new(0, 0, 1920, 1080);
        assert_eq!(
            snap(Rect::new(7, 970, 275, 116), &[], Some(work), 10),
            Rect::new(0, 964, 275, 116)
        );
    }

    #[test]
    fn flush_bottom_stays_flush() {
        let work = Rect::new(0, 0, 1920, 1080);
        let main = Rect::new(100, 964, 275, 116);
        let mut rects = [main];
        resize_propagate(
            &mut rects,
            0,
            main,
            Rect::new(100, 964, 275, 14),
            Some(work),
        );
        assert_eq!(rects[0].bottom(), 1080);
    }

    #[test]
    fn keep_on_screen_margin() {
        let work = Rect::new(0, 0, 800, 600);
        assert_eq!(keep_on_screen(Rect::new(-10, 0, 100, 100), work, 10).x, -10);
        assert_eq!(keep_on_screen(Rect::new(-11, 0, 100, 100), work, 10).x, 0);
        assert_eq!(keep_on_screen(Rect::new(750, 0, 100, 100), work, 10).x, 700);
    }
}
