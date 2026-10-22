//! The options tree (game/src/menu_tree.rs): every row once, every page
//! reachable and escapable, everything inside the 320x240 panel.

#[allow(dead_code)]
#[path = "../../../game/src/menu_tree.rs"]
mod menu_tree;

use menu_tree::*;

const ALL: [Page; PAGES] = [
    Page::Pause,
    Page::Options,
    Page::Video,
    Page::Sound,
    Page::Sticks,
    Page::Gameplay,
];

/// Every page of one tree (pause or title), root first.
fn tree(at_title: bool) -> Vec<Page> {
    ALL.iter()
        .copied()
        .filter(|&p| !(at_title && p == Page::Pause))
        .collect()
}

#[test]
fn no_label_appears_twice_in_a_tree() {
    for at_title in [false, true] {
        let mut seen: Vec<&str> = Vec::new();
        for page in tree(at_title) {
            for row in rows(page, at_title) {
                if row.kind == Kind::Done {
                    continue; // one DONE per page, as in Java
                }
                assert!(
                    !seen.contains(&row.label),
                    "{} shows twice (title: {at_title})",
                    row.label
                );
                seen.push(row.label);
            }
        }
    }
}

#[test]
fn flight_is_one_row_in_the_whole_game() {
    // The options tree has no flight row; the cheats screen it opens has one.
    for at_title in [false, true] {
        for page in tree(at_title) {
            for row in rows(page, at_title) {
                assert!(
                    !row.label.contains("FLIGHT") && !row.label.contains("FLY"),
                    "{}",
                    row.label
                );
            }
        }
    }
    let cheats = include_str!("../../../game/src/cheat.rs");
    let rows_start = cheats.find("pub const ROWS").expect("cheat ROWS");
    let rows_end = rows_start + cheats[rows_start..].find("];").unwrap();
    let flights = cheats[rows_start..rows_end].matches("\"FLY").count()
        + cheats[rows_start..rows_end].matches("\"FLIGHT").count();
    assert_eq!(flights, 1);
}

#[test]
fn every_sub_page_ends_in_done_and_the_first_pages_differ() {
    for at_title in [false, true] {
        for page in tree(at_title) {
            let last = rows(page, at_title).last().unwrap();
            if page == first(at_title) && !at_title {
                assert_ne!(
                    last.kind,
                    Kind::Done,
                    "the pause page leaves by BACK TO GAME"
                );
            } else {
                assert_eq!(last.kind, Kind::Done, "{page:?}");
            }
        }
    }
}

#[test]
fn every_page_climbs_back_to_the_first_page() {
    for at_title in [false, true] {
        for page in tree(at_title) {
            let mut at = page;
            for _ in 0..PAGES {
                match parent(at, at_title) {
                    Some(up) => at = up,
                    None => break,
                }
            }
            assert_eq!(at, first(at_title), "{page:?} (title: {at_title})");
            assert_eq!(parent(first(at_title), at_title), None);
        }
    }
}

#[test]
fn every_sub_page_is_entered_from_its_parent() {
    for at_title in [false, true] {
        for page in tree(at_title) {
            for row in rows(page, at_title) {
                if let Kind::Go(to) = row.kind {
                    assert_eq!(parent(to, at_title), Some(page), "{}", row.label);
                }
            }
            if let Some(up) = parent(page, at_title) {
                assert!(
                    rows(up, at_title).iter().any(|r| r.kind == Kind::Go(page)),
                    "{page:?} has no row on {up:?}"
                );
            }
        }
    }
}

#[test]
fn the_title_tree_has_only_rows_a_title_can_do() {
    for page in tree(true) {
        for row in rows(page, true) {
            assert!(
                !matches!(
                    row.kind,
                    Kind::Resume
                        | Kind::Controls
                        | Kind::SaveWorld
                        | Kind::LoadWorld
                        | Kind::SaveQuit
                        | Kind::Cheats
                ),
                "{} needs a world",
                row.label
            );
        }
    }
}

#[test]
fn each_live_setting_is_reachable_exactly_once() {
    for at_title in [false, true] {
        for setting in [
            S_MOVE_DEADZONE,
            S_LOOK_DEADZONE,
            S_LOOK_SPEED,
            S_INVERT_Y,
            S_SFX,
            S_MUSIC,
            S_BRIGHTNESS,
            S_SCREEN_X,
            S_SCREEN_Y,
        ] {
            let n = tree(at_title)
                .into_iter()
                .flat_map(|p| rows(p, at_title))
                .filter(|r| r.kind == Kind::Set(setting))
                .count();
            assert_eq!(n, 1, "setting {setting} (title: {at_title})");
        }
        let saves = tree(at_title)
            .into_iter()
            .flat_map(|p| rows(p, at_title))
            .filter(|r| r.kind == Kind::SaveSettings)
            .count();
        assert_eq!(saves, 1);
    }
}

#[test]
fn pages_fit_the_panel_and_the_rows_fit_their_buttons() {
    // Rows start at y 44 on a 16 px pitch, 14 px tall; the message line sits
    // at y 168 and the panel ends at 182. A button is 252 px wide with 8 px of
    // text inset; a value is up to 12 glyphs at the right.
    for at_title in [false, true] {
        for page in tree(at_title) {
            let rows = rows(page, at_title);
            let air: i32 = rows.iter().filter(|r| r.gap).count() as i32 * 8;
            let bottom = 44 + (rows.len() as i32 - 1) * 16 + air + 14;
            assert!(bottom <= 164, "{page:?} runs to y {bottom}");
            assert!(title_of(page, at_title).len() * 8 <= 240);
            for row in rows {
                let value = match row.kind {
                    Kind::Set(_) | Kind::SaveSettings | Kind::Tutorial => 12 * 8,
                    _ => 0,
                };
                assert!(
                    row.label.len() * 8 + value <= 252 - 16,
                    "{} does not fit",
                    row.label
                );
            }
        }
    }
}
