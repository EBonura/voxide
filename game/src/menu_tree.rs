//! The shape of the options tree (menus.rs draws and steers it): which pages
//! exist, which rows each holds, where O goes. Plain data and no game state,
//! so the host tests (tools/host-tests) walk it with rustc.
//!
//!   GAME MENU (pause only)   BACK TO GAME, OPTIONS..., CONTROLS, SAVE WORLD,
//!                            LOAD WORLD
//!   OPTIONS                  VIDEO SETTINGS..., MUSIC & SOUNDS...,
//!                            STICK SETTINGS..., GAMEPLAY..., SAVE SETTINGS,
//!                            DONE
//!
//! The title screen's SETTINGS card opens the same OPTIONS page and the same
//! four sub-pages; only the rows that need a world (CHEATS) are left off.

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Page {
    Pause,
    Options,
    Video,
    Sound,
    Sticks,
    Gameplay,
}
pub const PAGES: usize = 6;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Kind {
    /// X opens a sub-page.
    Go(Page),
    /// A live setting (main.rs `setting_adjust` row): LEFT and RIGHT step it.
    Set(usize),
    Tutorial,
    SaveSettings,
    /// Up one level; from the first page, out of the tree.
    Done,
    Resume,
    Controls,
    SaveWorld,
    LoadWorld,
    Cheats,
}

pub struct Row {
    pub label: &'static str,
    pub kind: Kind,
    /// Eight pixels of air above it: Java sets DONE apart from the list.
    pub gap: bool,
}

const fn row(label: &'static str, kind: Kind) -> Row {
    Row { label, kind, gap: false }
}
const fn apart(label: &'static str, kind: Kind) -> Row {
    Row { label, kind, gap: true }
}

// The live settings, in the order main.rs' `setting_adjust` and
// `setting_value` number them.
pub const S_MOVE_DEADZONE: usize = 0;
pub const S_LOOK_DEADZONE: usize = 1;
pub const S_LOOK_SPEED: usize = 2;
pub const S_INVERT_Y: usize = 3;
pub const S_SFX: usize = 4;
pub const S_MUSIC: usize = 5;
pub const S_BRIGHTNESS: usize = 6;
pub const S_SCREEN_X: usize = 7;
pub const S_SCREEN_Y: usize = 8;
/// The SAVE SETTINGS row of `setting_value`: the save status.
pub const S_SAVE: usize = 9;

const PAUSE: [Row; 5] = [
    row("BACK TO GAME", Kind::Resume),
    row("OPTIONS...", Kind::Go(Page::Options)),
    row("CONTROLS", Kind::Controls),
    row("SAVE WORLD", Kind::SaveWorld),
    row("LOAD WORLD", Kind::LoadWorld),
];
const OPTIONS: [Row; 6] = [
    row("VIDEO SETTINGS...", Kind::Go(Page::Video)),
    row("MUSIC & SOUNDS...", Kind::Go(Page::Sound)),
    row("STICK SETTINGS...", Kind::Go(Page::Sticks)),
    row("GAMEPLAY...", Kind::Go(Page::Gameplay)),
    row("SAVE SETTINGS", Kind::SaveSettings),
    apart("DONE", Kind::Done),
];
const VIDEO: [Row; 4] = [
    row("BRIGHTNESS", Kind::Set(S_BRIGHTNESS)),
    row("SCREEN X", Kind::Set(S_SCREEN_X)),
    row("SCREEN Y", Kind::Set(S_SCREEN_Y)),
    apart("DONE", Kind::Done),
];
const SOUND: [Row; 3] = [
    row("MUSIC VOLUME", Kind::Set(S_MUSIC)),
    row("SFX VOLUME", Kind::Set(S_SFX)),
    apart("DONE", Kind::Done),
];
const STICKS: [Row; 5] = [
    row("LOOK SPEED", Kind::Set(S_LOOK_SPEED)),
    row("INVERT LOOK Y", Kind::Set(S_INVERT_Y)),
    row("LOOK DEADZONE", Kind::Set(S_LOOK_DEADZONE)),
    row("MOVE DEADZONE", Kind::Set(S_MOVE_DEADZONE)),
    apart("DONE", Kind::Done),
];
const GAMEPLAY: [Row; 3] = [
    row("TUTORIAL", Kind::Tutorial),
    row("CHEATS...", Kind::Cheats),
    apart("DONE", Kind::Done),
];
/// The same page from the title screen: cheats act on a world that is not
/// there yet.
const GAMEPLAY_TITLE: [Row; 2] = [row("TUTORIAL", Kind::Tutorial), apart("DONE", Kind::Done)];

pub fn rows(page: Page, at_title: bool) -> &'static [Row] {
    match page {
        Page::Pause => &PAUSE,
        Page::Options => &OPTIONS,
        Page::Video => &VIDEO,
        Page::Sound => &SOUND,
        Page::Sticks => &STICKS,
        Page::Gameplay if at_title => &GAMEPLAY_TITLE,
        Page::Gameplay => &GAMEPLAY,
    }
}

pub fn title_of(page: Page, at_title: bool) -> &'static str {
    match page {
        Page::Pause => "GAME MENU",
        // The title screen's button says SETTINGS; the page keeps its name.
        Page::Options if at_title => "SETTINGS",
        Page::Options => "OPTIONS",
        Page::Video => "VIDEO SETTINGS",
        Page::Sound => "MUSIC & SOUNDS",
        Page::Sticks => "STICK SETTINGS",
        Page::Gameplay => "GAMEPLAY",
    }
}

/// Where O goes from `page`; None leaves the tree (closes the pause menu, or
/// returns to the title screen).
pub fn parent(page: Page, at_title: bool) -> Option<Page> {
    match page {
        Page::Pause => None,
        Page::Options if at_title => None,
        Page::Options => Some(Page::Pause),
        _ => Some(Page::Options),
    }
}

/// The page that opens when the tree does.
pub fn first(at_title: bool) -> Page {
    if at_title {
        Page::Options
    } else {
        Page::Pause
    }
}

/// True when LEFT and RIGHT do something on the page (its hint says so).
pub fn adjusts(page: Page, at_title: bool) -> bool {
    rows(page, at_title).iter().any(|r| matches!(r.kind, Kind::Set(_) | Kind::Tutorial))
}

/// True when the page's values are kept only by SAVE SETTINGS, which it says.
pub fn holds_settings(page: Page) -> bool {
    matches!(page, Page::Options | Page::Video | Page::Sound | Page::Sticks)
}
