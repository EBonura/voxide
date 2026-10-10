//! The pad grammar every container screen shares: the inventory's item tabs
//! and hotbar, the player page with its crafting grid, the chest and the
//! furnace. Minecraft's PlayStation editions (Legacy Console) set the
//! scheme and this is the PS1 mapping of it:
//!
//!   X         take the stack under the cursor, then place it (swap if the
//!             slot holds something else)
//!   SQUARE    take half the stack, then place one
//!   TRIANGLE  quick move: send the stack under the cursor to its other
//!             side without picking it up (output slot: craft them all)
//!   CIRCLE    put the stack in hand back, or close the screen when the
//!             hand is empty
//!
//! One table, `decide`, says what a button does for the slot under the
//! cursor. The input code runs the answer and the prompt bar at the bottom of
//! the screen prints it, so the two cannot disagree: a prompt only appears
//! for a button that does something here, and X never means two things.
//!
//! Plain data and `const`-free logic with no dependencies, so the host test
//! (`rustc --test` on this file) can pin the table.

/// What kind of slot the cursor is on.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Zone {
    /// A kind in an inventory tab's grid.
    Grid,
    /// One of the hotbar's slots.
    Hotbar,
    /// An armour slot on the player page.
    Armor,
    /// An armour piece you carry, under the player page's armour.
    Pack,
    /// The off hand.
    Off,
    /// One of the weapons you can swing.
    Weapon,
    /// A cell of the 2x2 crafting grid.
    Cell,
    /// The crafting grid's output slot.
    Result,
    /// Your pane of a chest.
    Mine,
    /// The chest's own pane.
    Theirs,
    /// Your pane of the furnace (what it can smelt or burn).
    MineFurnace,
    /// The furnace's input slot.
    FurnIn,
    /// The furnace's output slot.
    FurnOut,
    /// The furnace's fuel slot.
    FurnFuel,
}

/// The four face buttons a container screen uses.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Btn {
    Cross,
    Square,
    Triangle,
    Circle,
}

/// What a button does. `None` means the prompt is not shown.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Act {
    None,
    /// Pick the whole stack up.
    Take,
    /// Pick half of it up (rounded up).
    Half,
    /// Put the stack in hand here.
    Place,
    /// Put one of the stack in hand here.
    One,
    /// Put the stack in hand here and pick up what was there.
    Swap,
    /// Put an armour piece on.
    Wear,
    /// Choose this weapon.
    Equip,
    /// Quick move to the first free hotbar slot.
    ToHotbar,
    /// Quick move back into the pack (off the hotbar, out of a cell, off
    /// the body, out of the furnace or chest).
    ToPack,
    /// Quick move into the chest.
    ToChest,
    /// Quick move into the furnace.
    ToFurnace,
    /// Craft once.
    Craft,
    /// Craft until the cells run out.
    CraftAll,
    /// Put the stack in hand back where it came from.
    Back,
    /// Close the screen.
    Close,
}

/// The slot under the cursor and the hand.
#[derive(Copy, Clone, Debug)]
pub struct Spot {
    pub zone: Zone,
    /// Something is in the slot (for a weapon: you own it).
    pub full: bool,
    /// How many are in it.
    pub count: u16,
    /// A stack is in hand.
    pub holding: bool,
    /// The hand holds the same kind as the slot.
    pub same: bool,
    /// Chest and furnace panes: the hand was lifted from the other side.
    pub from_other: bool,
}

/// What `b` does with the cursor at `s`.
#[cfg_attr(not(test), optimize(size))]
pub fn decide(s: Spot, b: Btn) -> Act {
    if b == Btn::Circle {
        return if s.holding { Act::Back } else { Act::Close };
    }
    let (x, sq, tri) = (b == Btn::Cross, b == Btn::Square, b == Btn::Triangle);
    // The common shape of a slot that holds a stack you can lift and drop on.
    let swap_or_place = if s.full && !s.same {
        Act::Swap
    } else {
        Act::Place
    };
    // A stack you can take only when the hand is empty.
    let lift = !s.holding && s.full;
    let lift_half = lift && s.count >= 2;
    match s.zone {
        Zone::Grid => match () {
            _ if x && lift => Act::Take,
            _ if sq && lift_half => Act::Half,
            _ if tri && s.full => Act::ToHotbar,
            _ => Act::None,
        },
        Zone::Hotbar => match () {
            _ if x && s.holding => swap_or_place,
            _ if x && lift => Act::Take,
            _ if tri && s.full => Act::ToPack,
            _ => Act::None,
        },
        Zone::Armor => match () {
            _ if x && s.holding => Act::Wear,
            _ if tri && s.full => Act::ToPack,
            _ => Act::None,
        },
        Zone::Pack => match () {
            _ if x && lift => Act::Take,
            _ if tri && s.full => Act::Wear,
            _ => Act::None,
        },
        Zone::Off => match () {
            _ if x && s.holding => swap_or_place,
            _ if x && lift => Act::Take,
            _ if tri && s.full => Act::ToPack,
            _ => Act::None,
        },
        Zone::Weapon => match () {
            _ if x && s.full => Act::Equip,
            _ => Act::None,
        },
        Zone::Cell => match () {
            _ if x && s.holding => swap_or_place,
            _ if x && lift => Act::Take,
            // One onto an empty cell or the same kind; a different kind swaps.
            _ if sq && s.holding && (!s.full || s.same) => Act::One,
            _ if sq && lift_half => Act::Half,
            _ if tri && s.full => Act::ToPack,
            _ => Act::None,
        },
        Zone::Result => match () {
            _ if x && s.full => Act::Craft,
            _ if tri && s.full => Act::CraftAll,
            _ => Act::None,
        },
        // A pane the stack can be lifted from, and dropped on when the hand
        // came from the other side.
        Zone::Mine | Zone::Theirs | Zone::MineFurnace => {
            let drop = s.holding && s.from_other;
            let quick = match s.zone {
                Zone::Mine => Act::ToChest,
                Zone::MineFurnace => Act::ToFurnace,
                _ => Act::ToPack,
            };
            match () {
                _ if x && drop => Act::Place,
                _ if x && lift => Act::Take,
                _ if sq && drop => Act::One,
                _ if sq && lift_half => Act::Half,
                _ if tri && s.full => quick,
                _ => Act::None,
            }
        }
        Zone::FurnIn => {
            let drop = s.holding && s.from_other;
            match () {
                _ if x && drop => Act::Place,
                _ if x && lift => Act::Take,
                _ if sq && drop => Act::One,
                _ if sq && lift_half => Act::Half,
                _ if tri && s.full => Act::ToPack,
                _ => Act::None,
            }
        }
        // Collect only: nothing is put in the output.
        Zone::FurnOut => match () {
            _ if x && lift => Act::Take,
            _ if sq && lift_half => Act::Half,
            _ if tri && s.full => Act::ToPack,
            _ => Act::None,
        },
        // Fuel goes in and is burnt: it does not come back.
        Zone::FurnFuel => {
            let drop = s.holding && s.from_other;
            match () {
                _ if x && drop => Act::Place,
                _ if sq && drop => Act::One,
                _ => Act::None,
            }
        }
    }
}

/// The prompt for an action, in the console-edition style: one or two words.
#[cfg_attr(not(test), optimize(size))]
pub fn label(a: Act) -> &'static str {
    match a {
        Act::None => "",
        Act::Take => "TAKE",
        Act::Half => "HALF",
        Act::Place => "PLACE",
        Act::One => "ONE",
        Act::Swap => "SWAP",
        Act::Wear => "WEAR",
        Act::Equip => "EQUIP",
        Act::ToHotbar => "TO HOTBAR",
        Act::ToPack => "TO PACK",
        Act::ToChest => "TO CHEST",
        Act::ToFurnace => "TO FURNACE",
        Act::Craft => "CRAFT",
        Act::CraftAll => "CRAFT ALL",
        Act::Back => "BACK",
        Act::Close => "CLOSE",
    }
}

/// Half of a stack, rounded up: what SQUARE picks up.
pub fn half(n: u16) -> u16 {
    n - n / 2
}

#[cfg(test)]
mod tests {
    use super::*;

    const ZONES: [Zone; 14] = [
        Zone::Grid,
        Zone::Hotbar,
        Zone::Armor,
        Zone::Pack,
        Zone::Off,
        Zone::Weapon,
        Zone::Cell,
        Zone::Result,
        Zone::Mine,
        Zone::Theirs,
        Zone::MineFurnace,
        Zone::FurnIn,
        Zone::FurnOut,
        Zone::FurnFuel,
    ];
    const BTNS: [Btn; 4] = [Btn::Cross, Btn::Square, Btn::Triangle, Btn::Circle];

    /// Every state a cursor can be in on any zone.
    fn all(mut f: impl FnMut(Spot, Btn)) {
        for zone in ZONES {
            for full in [false, true] {
                for count in [0u16, 1, 2, 63] {
                    for holding in [false, true] {
                        for same in [false, true] {
                            for from_other in [false, true] {
                                let s = Spot {
                                    zone,
                                    full,
                                    count,
                                    holding,
                                    same,
                                    from_other,
                                };
                                for b in BTNS {
                                    f(s, b);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn circle_puts_back_when_holding_and_closes_otherwise() {
        all(|s, b| {
            if b == Btn::Circle {
                assert_eq!(decide(s, b), if s.holding { Act::Back } else { Act::Close });
            } else {
                // Back and Close live on CIRCLE and nowhere else.
                assert!(
                    !matches!(decide(s, b), Act::Back | Act::Close),
                    "{s:?} {b:?}"
                );
            }
        });
    }

    #[test]
    fn x_never_puts_back_and_never_does_two_things_in_one_state() {
        // The fault this table replaces: X both put a stack on a hotbar slot
        // and put it back in the grid. X now takes, places or swaps; whatever
        // returns a stack is CIRCLE (hand) or TRIANGLE (quick move).
        all(|s, b| {
            if b == Btn::Cross {
                let a = decide(s, b);
                assert!(
                    matches!(
                        a,
                        Act::None
                            | Act::Take
                            | Act::Place
                            | Act::Swap
                            | Act::Wear
                            | Act::Equip
                            | Act::Craft
                    ),
                    "{s:?} gave X {a:?}"
                );
                // A stack can be picked up or put down, not both at once.
                if s.holding {
                    assert_ne!(a, Act::Take, "{s:?}");
                } else {
                    assert!(!matches!(a, Act::Place | Act::Swap), "{s:?}");
                }
            }
        });
    }

    #[test]
    fn an_empty_slot_with_an_empty_hand_offers_nothing_but_close() {
        all(|s, b| {
            if !s.full && !s.holding && b != Btn::Circle {
                assert_eq!(decide(s, b), Act::None, "{s:?} {b:?}");
            }
        });
    }

    #[test]
    fn square_halves_a_stack_of_two_or_more_and_places_one() {
        all(|s, b| {
            if b != Btn::Square {
                return;
            }
            match decide(s, b) {
                Act::None => {}
                Act::Half => assert!(!s.holding && s.full && s.count >= 2, "{s:?}"),
                Act::One => assert!(s.holding, "{s:?}"),
                a => panic!("{s:?} gave SQUARE {a:?}"),
            }
        });
    }

    #[test]
    fn triangle_is_quick_move_and_never_uses_the_hand() {
        all(|s, b| {
            if b != Btn::Triangle {
                return;
            }
            let a = decide(s, b);
            assert!(
                matches!(
                    a,
                    Act::None
                        | Act::ToHotbar
                        | Act::ToPack
                        | Act::ToChest
                        | Act::ToFurnace
                        | Act::Wear
                        | Act::CraftAll
                ),
                "{s:?} gave TRIANGLE {a:?}"
            );
            // It acts on a stack that is there, held or not.
            if a != Act::None {
                assert!(s.full, "{s:?}");
            }
        });
    }

    #[test]
    fn grid_to_hotbar_then_hotbar_to_pack_round_trips() {
        let grid = Spot {
            zone: Zone::Grid,
            full: true,
            count: 9,
            holding: false,
            same: false,
            from_other: false,
        };
        let hot = Spot {
            zone: Zone::Hotbar,
            ..grid
        };
        assert_eq!(decide(grid, Btn::Triangle), Act::ToHotbar);
        assert_eq!(decide(hot, Btn::Triangle), Act::ToPack);
    }

    #[test]
    fn swap_only_where_the_slot_holds_something_else() {
        let mut s = Spot {
            zone: Zone::Hotbar,
            full: true,
            count: 5,
            holding: true,
            same: false,
            from_other: false,
        };
        assert_eq!(decide(s, Btn::Cross), Act::Swap);
        s.same = true;
        assert_eq!(decide(s, Btn::Cross), Act::Place);
        s.full = false;
        s.same = false;
        assert_eq!(decide(s, Btn::Cross), Act::Place);
    }

    #[test]
    fn chest_stacks_cross_between_panes_only() {
        let mut mine = Spot {
            zone: Zone::Mine,
            full: true,
            count: 40,
            holding: false,
            same: false,
            from_other: false,
        };
        assert_eq!(decide(mine, Btn::Cross), Act::Take);
        assert_eq!(decide(mine, Btn::Square), Act::Half);
        assert_eq!(decide(mine, Btn::Triangle), Act::ToChest);
        let theirs = Spot {
            zone: Zone::Theirs,
            ..mine
        };
        assert_eq!(decide(theirs, Btn::Triangle), Act::ToPack);
        // Holding a stack from your own pane: it can go to the chest, and
        // dropping it on your own pane does nothing (CIRCLE puts it back).
        mine.holding = true;
        assert_eq!(decide(mine, Btn::Cross), Act::None);
        let mut chest = Spot {
            from_other: true,
            ..theirs
        };
        chest.holding = true;
        assert_eq!(decide(chest, Btn::Cross), Act::Place);
        assert_eq!(decide(chest, Btn::Square), Act::One);
    }

    #[test]
    fn furnace_output_is_collect_only_and_fuel_does_not_come_back() {
        let out = Spot {
            zone: Zone::FurnOut,
            full: true,
            count: 8,
            holding: true,
            same: false,
            from_other: true,
        };
        assert_eq!(decide(out, Btn::Cross), Act::None);
        let out = Spot {
            holding: false,
            from_other: false,
            ..out
        };
        assert_eq!(decide(out, Btn::Cross), Act::Take);
        assert_eq!(decide(out, Btn::Triangle), Act::ToPack);
        let fuel = Spot {
            zone: Zone::FurnFuel,
            full: true,
            count: 8,
            holding: false,
            same: false,
            from_other: false,
        };
        assert_eq!(decide(fuel, Btn::Cross), Act::None);
        assert_eq!(decide(fuel, Btn::Triangle), Act::None);
        let fuel = Spot {
            holding: true,
            from_other: true,
            ..fuel
        };
        assert_eq!(decide(fuel, Btn::Cross), Act::Place);
    }

    #[test]
    fn the_result_slot_crafts_one_or_all() {
        let r = Spot {
            zone: Zone::Result,
            full: true,
            count: 4,
            holding: false,
            same: false,
            from_other: false,
        };
        assert_eq!(decide(r, Btn::Cross), Act::Craft);
        assert_eq!(decide(r, Btn::Triangle), Act::CraftAll);
        assert_eq!(decide(r, Btn::Square), Act::None);
    }

    #[test]
    fn every_prompt_fits_the_bar() {
        // X, SQUARE and TRIANGLE share the first prompt line, from x = 16 to
        // the panel's right edge (304); CIRCLE leads the second line with the
        // page buttons. hint_item lays each as a pill (8 px a glyph + 7), 3 px,
        // the label at 8 px a character, and 8 px of spacing.
        let mut widest = 0;
        all(|s, _| {
            let mut w = 0;
            for (b, glyphs) in [(Btn::Cross, 1), (Btn::Square, 2), (Btn::Triangle, 1)] {
                let a = decide(s, b);
                let l = label(a);
                if a != Act::None {
                    assert!(l.is_ascii() && l == l.to_ascii_uppercase());
                    w += glyphs * 8 + 7 + 3 + l.len() as i32 * 8 + 8;
                }
            }
            widest = widest.max(w);
        });
        assert!(16 + widest <= 304, "{widest}");
        // CIRCLE's two labels leave the second line room for the page buttons.
        assert!(label(Act::Back).len() <= 5 && label(Act::Close).len() <= 5);
    }

    #[test]
    fn half_rounds_up() {
        assert_eq!(half(1), 1);
        assert_eq!(half(2), 1);
        assert_eq!(half(3), 2);
        assert_eq!(half(64), 32);
    }
}
