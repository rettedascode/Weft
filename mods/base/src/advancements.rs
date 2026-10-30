//! Advancements: milestones of a survival game, from the first log to the top of a mountain.
//! The server watches events and the players' inventories and grants them; a player's client
//! shows a toast for each new one and lists them all on a screen (L, or the pause menu).

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Instant;

use weft_sdk::*;

use crate::items::ARMOR;
use crate::mobs::{COW, PIG, SPIDER, ZOMBIE};

pub use base_api::{ADVANCEMENTS, Advancements};

/// What a player has to do. Inventory and position rules are checked once a second, so
/// anything that ends up in the inventory counts, however it got there.
enum Rule {
    Have(&'static Item),
    HaveTag(&'static Tag),
    /// Breaks a block with this tag.
    Break(&'static Tag),
    /// Places a block with this tag.
    Place(&'static Tag),
    Kill(&'static [&'static Entity]),
    Eat(&'static [&'static Item]),
    /// Wears at least this many pieces of armor.
    Wear(u32),
    Below(i32),
    /// Standing on the ground this high up.
    Above(i32),
}

struct Advancement {
    /// `advancement.base.<id>` and `.desc` in the lang files.
    id: &'static str,
    icon: &'static str,
    /// Index into `GROUPS`: the screen shows each group as a chain, in the order of `ALL`.
    group: usize,
    rule: Rule,
}

/// `advancements.base.group.<name>` in the lang files.
const GROUPS: [&str; 4] = ["start", "progress", "food_and_combat", "exploration"];

static LOGS: Tag = Tag::named("logs");
static TORCHES: Tag = Tag::named("torches");
static PICKAXES: Tag = Tag::named("tools/pickaxe");
static SWORDS: Tag = Tag::named("tools/sword");
static CRAFTING_TABLE: Item = Item::named("crafting_table");
static FURNACE: Item = Item::named("furnace");
static COBBLESTONE: Item = Item::named("cobblestone");
static BREAD: Item = Item::named("bread");
static LAVA_BUCKET: Item = Item::named("lava_bucket");
static COOKED_BEEF: Item = Item::named("cooked_beef");
static COOKED_PORKCHOP: Item = Item::named("cooked_porkchop");

/// In the order they usually come. The index is the bit in `Advancements::done`, so new ones go
/// at the end (at most 64).
static ALL: [Advancement; 19] = [
    Advancement { id: "timber", icon: "base:oak_log", group: 0, rule: Rule::Break(&LOGS) },
    Advancement { id: "workbench", icon: "base:crafting_table", group: 0, rule: Rule::Have(&CRAFTING_TABLE) },
    Advancement { id: "first_pickaxe", icon: "base:wood_pickaxe", group: 0, rule: Rule::HaveTag(&PICKAXES) },
    Advancement { id: "stone_age", icon: "base:cobblestone", group: 0, rule: Rule::Have(&COBBLESTONE) },
    Advancement { id: "armed", icon: "base:stone_sword", group: 0, rule: Rule::HaveTag(&SWORDS) },
    Advancement { id: "furnace", icon: "base:furnace", group: 0, rule: Rule::Have(&FURNACE) },
    Advancement { id: "light", icon: "base:torch", group: 0, rule: Rule::Place(&TORCHES) },
    Advancement { id: "iron", icon: "base:iron_ingot", group: 1, rule: Rule::Have(&crate::items::IRON_INGOT) },
    Advancement { id: "gold", icon: "base:gold_ingot", group: 1, rule: Rule::Have(&crate::items::GOLD_INGOT) },
    Advancement { id: "suit_up", icon: "base:iron_chestplate", group: 1, rule: Rule::Wear(1) },
    Advancement { id: "full_armor", icon: "base:diamond_chestplate", group: 1, rule: Rule::Wear(4) },
    Advancement { id: "diamonds", icon: "base:diamond", group: 1, rule: Rule::Have(&crate::items::DIAMOND) },
    Advancement { id: "baker", icon: "base:bread", group: 2, rule: Rule::Have(&BREAD) },
    Advancement { id: "hunter", icon: "base:porkchop", group: 2, rule: Rule::Kill(&[&PIG, &COW]) },
    Advancement { id: "cook", icon: "base:cooked_beef", group: 2, rule: Rule::Eat(&[&COOKED_BEEF, &COOKED_PORKCHOP]) },
    Advancement { id: "monster_hunter", icon: "base:rotten_flesh", group: 2, rule: Rule::Kill(&[&ZOMBIE, &SPIDER]) },
    Advancement { id: "hot_stuff", icon: "base:lava_bucket", group: 1, rule: Rule::Have(&LAVA_BUCKET) },
    Advancement { id: "deep", icon: "base:diamond_ore", group: 3, rule: Rule::Below(0) },
    Advancement { id: "summit", icon: "base:snow", group: 3, rule: Rule::Above(180) },
];

/// Server to the player who made it: the index into `ALL`.
static MADE: Channel<u32> = Channel::named("advancement_made");
static CHIME: Sound = Sound::named("ui.advancement");
static SCREEN: Screen = Screen::named("advancements");
static OPEN: Key = Key::named("advancements");

const TOAST_SECS: f32 = 5.0;
const SLIDE_SECS: f32 = 0.35;
/// The toast's box (`gui/advancement_toast.png`) in GUI pixels, and its gap to the window's edge.
const TOAST_W: f32 = 160.0;
const TOAST_MARGIN: f32 = 4.0;
/// Top of the icon in the box, centered in its slot (y 7 to 27).
const ICON_Y: f32 = 9.0;

pub fn register(r: &mut Registry) {
    r.entity_component(&ADVANCEMENTS, Advancements { done: 0 }, component_flags::SAVE | component_flags::SYNC_OWNER);
    r.attach(&PLAYER, &ADVANCEMENTS);
    r.sound(&CHIME).files(&["random/advancement"]).category(&sound_categories::UI);

    r.world_tick(check_players);
    r.on(&BLOCK_BROKEN, |b| {
        let w = b.world();
        for c in b.iter() {
            grant_where(&w, c.player, |rule| matches!(rule, Rule::Break(t) if t.on_block(c.state)));
        }
    });
    r.on(&BLOCK_PLACED, |b| {
        let w = b.world();
        for c in b.iter() {
            grant_where(&w, c.player, |rule| matches!(rule, Rule::Place(t) if t.on_block(c.state)));
        }
    });
    r.on(&ENTITY_DIED, |b| {
        let w = b.world();
        for d in b.iter() {
            grant_where(&w, d.killer, |rule| matches!(rule, Rule::Kill(kinds) if kinds.iter().any(|k| k.id() == d.kind)));
        }
    });
    r.on(&ITEM_USED, |b| {
        let w = b.world();
        for u in b.iter() {
            grant_where(&w, u.player, |rule| matches!(rule, Rule::Eat(foods) if foods.iter().any(|f| f.id() == u.stack.item)));
        }
    });

    r.command("advancements", "[action:word(reset,all)]", |cmd| {
        let w = cmd.world();
        let Some(a) = w.component_mut(cmd.player(), &ADVANCEMENTS) else { return };
        match cmd.text("action") {
            Some("reset") => a.done = 0,
            Some("all") => a.done = (1u64 << ALL.len()) - 1,
            _ => {}
        }
        let done = a.done.count_ones() as i64;
        cmd.reply("command.base.advancements", &[Arg::Int(done), Arg::Int(ALL.len() as i64)]);
    });

    client(r);
}

/// Inventory, armor and height, once a second per player.
fn check_players(w: &World) {
    for id in w.players() {
        let Some(st) = w.entity(id) else { continue };
        if st.age % 20 != 0 || w.creative(id) {
            continue;
        }
        let inv = w.inventory(id);
        let items: Vec<u32> = (0..inv.size()).map(|i| inv.get(i)).filter(|s| !s.is_empty()).map(|s| s.item).collect();
        let worn = inv.find_group(EQUIPMENT_GROUP).map_or(0, |g| (0..g.len()).filter(|&i| ARMOR.on_item(g.get(i).item).is_some()).count() as u32);
        let y = st.pos[1];
        grant_where(w, id, |rule| match *rule {
            Rule::Have(item) => items.contains(&item.id()),
            Rule::HaveTag(tag) => items.iter().any(|&i| tag.on_item(i)),
            Rule::Wear(n) => worn >= n,
            Rule::Below(min) => y < min as f64,
            Rule::Above(max) => y >= max as f64 && st.on_ground != 0,
            _ => false,
        });
    }
}

/// Grants every advancement whose rule `met` says so to `player`, if it is a player in survival
/// that doesn't have it yet, and tells their client.
fn grant_where(w: &World, player: u64, met: impl Fn(&Rule) -> bool) {
    if player == 0 || w.creative(player) {
        return;
    }
    let Some(a) = w.component_mut(player, &ADVANCEMENTS) else { return };
    for (i, adv) in ALL.iter().enumerate() {
        let bit = 1u64 << i;
        if a.done & bit == 0 && met(&adv.rule) {
            a.done |= bit;
            w.send(player, &MADE, &(i as u32));
        }
    }
}

/// Toasts waiting to be shown, the first one with the moment it appeared.
static TOASTS: Mutex<VecDeque<(u32, Option<Instant>)>> = Mutex::new(VecDeque::new());

fn client(r: &mut Registry) {
    r.client_packet(&MADE, |_, i| TOASTS.lock().unwrap().push_back((i, None)));
    r.client_frame(|c| {
        let mut toasts = TOASTS.lock().unwrap();
        let Some((i, shown)) = toasts.front_mut() else { return };
        let Some(adv) = ALL.get(*i as usize) else {
            toasts.pop_front();
            return;
        };
        let start = *shown.get_or_insert_with(|| {
            c.play_sound(&CHIME, None, 1.0);
            Instant::now()
        });
        let t = start.elapsed().as_secs_f32();
        if t > TOAST_SECS {
            toasts.pop_front();
            return;
        }
        // Slides in from the right edge, eased, and back out at the end. Screen text is placed by
        // fractions of the window, centered on x with its top at y, so GUI pixels are converted.
        let shown = (t / SLIDE_SECS).min((TOAST_SECS - t) / SLIDE_SECS).clamp(0.0, 1.0);
        let ease = 1.0 - (1.0 - shown).powi(3);
        let st = c.state();
        let scale = st.gui_scale.max(1.0);
        let (gw, gh) = (st.window[0] as f32 / scale, st.window[1] as f32 / scale);
        let left = gw - (TOAST_W + TOAST_MARGIN) * ease;
        let text = |x: f32, y: f32, key: &str, args: &[Arg], color: u32, size: f32| c.screen_text((left + x) / gw, (TOAST_MARGIN + y) / gh, key, args, color, size);
        text(TOAST_W / 2.0, 0.0, "advancements.base.toast_box", &[], 0xffffffff, 1.0);
        // An icon in text is as tall as a line: twice the size fills the slot.
        text(16.0, ICON_Y, "", &[Arg::Item(Item::lookup(adv.icon).unwrap_or(0))], 0xffffffff, 2.0);
        // The text sits centered in the box right of the icon slot.
        let mid = 30.0 + (TOAST_W - 34.0) / 2.0;
        text(mid, 6.0, "advancement.base.toast", &[], 0xffe070ff, 1.0);
        text(mid, 17.0, &format!("advancement.base.{}", adv.id), &[], 0xffffffff, 1.0);
    });

    // Every group a chain of tiles, gold framed once made, with the details in their tooltips.
    // Built when the screen opens, from what the player has made by then.
    r.screen(&SCREEN, |ui| {
        let done = ui.client().component(&ADVANCEMENTS).map_or(0, |a| a.done);
        ui.panel("panel", S::col().pad(8.0).gap(6.0), |ui| {
            ui.row(S::row().gap(8.0), |ui| {
                ui.text_styled(S::col().class("title").scale(1.5).no_wrap(), "advancements.base.title", &[]);
                ui.spacer();
                ui.text_bind_styled(S::col().class("dim").no_wrap(), |v, out| {
                    let done = v.component(&ADVANCEMENTS).map_or(0, |a| a.done.count_ones());
                    out.set("advancements.base.progress", &[Arg::Int(done as i64), Arg::Int(ALL.len() as i64)]);
                });
            });
            ui.panel("well", S::col().pad(6.0).gap(6.0), |ui| {
                for (g, group) in GROUPS.iter().enumerate() {
                    ui.row(S::row(), |ui| {
                        ui.text_styled(S::col().w(90.0).no_wrap(), &format!("advancements.base.group.{group}"), &[]);
                        for (n, (i, adv)) in ALL.iter().enumerate().filter(|(_, a)| a.group == g).enumerate() {
                            let made = done & (1 << i) != 0;
                            if n > 0 {
                                ui.image(if made { "base:gui/advancement_line_done" } else { "base:gui/advancement_line" }, 6.0, 2.0);
                            }
                            let (title, desc) = (format!("advancement.base.{}", adv.id), format!("advancement.base.{}.desc", adv.id));
                            let status = if made { "advancements.base.done" } else { "advancements.base.locked" };
                            let tile = S::row().size(26.0, 26.0).center().tooltip_args("advancements.base.tip", &[Arg::Key(&title), Arg::Key(&desc), Arg::Key(status)]);
                            ui.panel(if made { "advancement_tile_done" } else { "advancement_tile" }, tile, |ui| {
                                ui.item(Item::lookup(adv.icon).unwrap_or(0), 1, S::default());
                            });
                        }
                    });
                }
            });
        });
    });
    r.key(&OPEN, "L").on_press(|c| {
        c.open_screen(&SCREEN);
    });
    r.screen_part(&screens::PAUSE, "buttons", 0, |ui| {
        ui.button("advancements.base.button", S::default(), |c| {
            c.client().open_screen(&SCREEN);
        });
    });
}
