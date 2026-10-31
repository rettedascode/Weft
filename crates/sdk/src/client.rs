//! Client side for mods: frame and screen hooks that see the local game and may drive it
//! (simulated input, the local player, worlds, screens, screenshots), mod key bindings, and
//! message channels between a mod's client and server parts.
//!
//! ```ignore
//! static JETPACK: Key = Key::named("jetpack");
//! r.key(&JETPACK, "J").on_press_server(|p| { let w = p.world(); /* for p.player() */ });
//! static SCAN: Key = Key::named("scan");
//! r.key(&SCAN, "K").on_press(|c| { /* on this client */ });
//! ```

use crate::abi::{self, ItemStack, Slice, Str, gizmo_kind as gk};
use crate::{ClientView, Key, Registry, Screen, Value, World, callback, host};

pub use abi::{ClientState, client_flags};

/// The local client during a client callback. Only valid for the duration of that callback.
#[derive(Clone, Copy)]
pub struct Client {
    raw: *mut abi::ClientCtx,
}

fn s(x: &str) -> Str {
    Str::new(x)
}

impl Client {
    pub(crate) fn from_raw(raw: *mut abi::ClientCtx) -> Client {
        Client { raw }
    }

    pub(crate) fn raw(&self) -> *mut abi::ClientCtx {
        self.raw
    }

    /// The read-only view UI bindings get (own player, components, menu data).
    pub fn view(&self) -> ClientView {
        ClientView::from_raw(self.raw)
    }

    pub fn state(&self) -> ClientState {
        let mut st = ClientState::DEFAULT;
        unsafe { (host().client_state)(self.raw, &mut st) };
        st
    }

    /// Top screen name (`core:menu`, `core:pause`, ...), empty in the world.
    pub fn screen(&self) -> String {
        unsafe { self.state().screen.as_str().to_string() }
    }

    fn input(&self, op: u32, text: &str, x: f32, y: f32) -> bool {
        unsafe { (host().client_input)(self.raw, op, s(text), x, y) != 0 }
    }

    /// Presses the key bound to `action` for `secs` (0 = a tap). False for an unknown action.
    pub fn press(&self, action: &str, secs: f32) -> bool {
        self.input(abi::client_input::ACTION, action, secs, 0.0)
    }

    /// A key by its settings name (`Enter`, `Tab`, `KeyE`, `F3`).
    pub fn key(&self, key: &str, secs: f32) -> bool {
        self.input(abi::client_input::KEY, key, secs, 0.0)
    }

    /// Mouse button 0 left, 1 right, 2 middle.
    pub fn click(&self, button: u32, secs: f32) {
        self.input(abi::client_input::CLICK, "", secs, button as f32);
    }

    /// Moves the mouse to fractions of the window.
    pub fn cursor(&self, x: f32, y: f32) {
        self.input(abi::client_input::CURSOR, "", x, y);
    }

    pub fn scroll(&self, steps: f32) {
        self.input(abi::client_input::SCROLL, "", steps, 0.0);
    }

    /// Text typed into the focused field.
    pub fn type_text(&self, text: &str) {
        self.input(abi::client_input::TYPE, text, 0.0, 0.0);
    }

    pub fn release_all(&self) {
        self.input(abi::client_input::RELEASE_ALL, "", 0.0, 0.0);
    }

    /// The key (yours from `Registry::key`, or one of `keys::*`) was pressed this frame. Keys
    /// belong to the world: while a screen has the input they are up.
    pub fn pressed(&self, key: &Key) -> bool {
        unsafe { (host().client_pressed)(self.raw, s(key.name())) & 1 != 0 }
    }

    pub fn held(&self, key: &Key) -> bool {
        unsafe { (host().client_pressed)(self.raw, s(key.name())) & 2 != 0 }
    }

    pub fn control(&self, c: &abi::PlayerControl) {
        unsafe { (host().client_set_player)(self.raw, c) }
    }

    /// Turns the local player, in degrees like the F3 screen.
    pub fn look(&self, yaw_deg: f32, pitch_deg: f32) {
        self.control(&abi::PlayerControl { mask: abi::control_mask::LOOK, yaw: yaw_deg.to_radians(), pitch: pitch_deg.to_radians(), ..abi::PlayerControl::DEFAULT });
    }

    /// Puts the local player's feet at `pos` (the server takes it as a move).
    pub fn teleport(&self, pos: [f64; 3]) {
        self.control(&abi::PlayerControl { mask: abi::control_mask::POS | abi::control_mask::VEL, pos, ..abi::PlayerControl::DEFAULT });
    }

    pub fn velocity(&self, vel: [f64; 3]) {
        self.control(&abi::PlayerControl { mask: abi::control_mask::VEL, vel, ..abi::PlayerControl::DEFAULT });
    }

    pub fn fly(&self, on: bool) {
        self.control(&abi::PlayerControl { mask: abi::control_mask::FLYING, flying: on as u32, ..abi::PlayerControl::DEFAULT });
    }

    pub fn fly_speed(&self, mul: f32) {
        self.control(&abi::PlayerControl { mask: abi::control_mask::FLY_SPEED, fly_speed: mul, ..abi::PlayerControl::DEFAULT });
    }

    /// Hotbar slot 0..9.
    pub fn select(&self, slot: u32) {
        self.control(&abi::PlayerControl { mask: abi::control_mask::SELECTED, selected: slot, ..abi::PlayerControl::DEFAULT });
    }

    fn command(&self, op: u32, text: &str, num: f64) -> u64 {
        unsafe { (host().client_command)(self.raw, op, s(text), num) }
    }

    /// A chat line or `/command`, as if typed.
    pub fn chat(&self, line: &str) {
        self.command(abi::client_op::CHAT, line, 0.0);
    }

    /// Captures the next frame to `path` (a bare name goes into `screenshots/`). The pixels are
    /// available a frame later through `image(id)`.
    pub fn screenshot(&self, path: &str) -> u64 {
        self.command(abi::client_op::SCREENSHOT, path, 0.0)
    }

    pub fn quit(&self, code: i32) {
        self.command(abi::client_op::QUIT, "", code as f64);
    }

    pub fn open_world(&self, name: &str, seed: u64) {
        self.command(abi::client_op::OPEN_WORLD, name, seed as f64);
    }

    /// Opens a fresh world, deleting a save of the same name first.
    pub fn new_world(&self, name: &str, seed: u64) {
        self.command(abi::client_op::NEW_WORLD, name, seed as f64);
    }

    pub fn leave_world(&self) {
        self.command(abi::client_op::LEAVE_WORLD, "", 0.0);
    }

    pub fn host_world(&self) {
        self.command(abi::client_op::HOST, "", 0.0);
    }

    pub fn join(&self, ticket: &str) {
        self.command(abi::client_op::JOIN, ticket, 0.0);
    }

    /// A mod's screen or one of the engine's (`screens::*`).
    pub fn open_screen(&self, screen: &Screen) -> bool {
        self.command(abi::client_op::OPEN_SCREEN, screen.name(), 0.0) != 0
    }

    pub fn close_screen(&self) {
        self.command(abi::client_op::CLOSE_SCREEN, "", 0.0);
    }

    pub fn reload_assets(&self) {
        self.command(abi::client_op::RELOAD_ASSETS, "", 0.0);
    }

    /// Statistics over `secs`, logged as a `BENCH` line; numbers in `state().bench_*` once
    /// `state().benches` goes up.
    pub fn bench(&self, secs: f32, label: &str) {
        self.command(abi::client_op::BENCH, label, secs as f64);
    }

    pub fn respawn(&self) {
        self.command(abi::client_op::RESPAWN, "", 0.0);
    }

    /// Times the field of view by `scale` (1 = as the player set it; less zooms in, like a drawn
    /// bow), eased in like the widening when sprinting. It stays until changed.
    pub fn fov_scale(&self, scale: f32) {
        self.command(abi::client_op::FOV_SCALE, "", scale as f64);
    }

    fn text(&self, which: u32, index: u32) -> String {
        unsafe { (host().client_text)(self.raw, which, index).as_str().to_string() }
    }

    pub fn ticket(&self) -> String {
        self.text(abi::client_text::TICKET, 0)
    }

    pub fn run_dir(&self) -> String {
        self.text(abi::client_text::RUN_DIR, 0)
    }

    /// Chat line `index` back from the newest, empty past the end.
    pub fn chat_line(&self, index: u32) -> String {
        self.text(abi::client_text::CHAT, index)
    }

    pub fn bench_line(&self) -> String {
        self.text(abi::client_text::BENCH, 0)
    }

    /// Text of the top screen's focused input field.
    pub fn input_text(&self) -> String {
        self.text(abi::client_text::INPUT, 0)
    }

    /// The block the client has at `pos`, `None` while that cube is not loaded.
    pub fn block(&self, pos: [i32; 3]) -> Option<u32> {
        let b = unsafe { (host().client_block)(self.raw, &pos) };
        (b != u32::MAX).then_some(b)
    }

    /// The light the client has at `pos` (sky level in the low byte, then block red, green,
    /// blue), `None` while that cube is not loaded.
    pub fn light(&self, pos: [i32; 3]) -> Option<u32> {
        let l = unsafe { (host().client_light)(self.raw, &pos) };
        (l != u32::MAX).then_some(l)
    }

    /// Own inventory slot by index (the engine lays the inventory out from its groups; with the
    /// base game the hotbar comes first).
    pub fn inventory_slot(&self, slot: u32) -> ItemStack {
        unsafe { (host().client_slot)(self.raw, 0, slot) }
    }

    pub fn menu_slot(&self, slot: u32) -> ItemStack {
        unsafe { (host().client_slot)(self.raw, 1, slot) }
    }

    pub fn cursor_stack(&self) -> ItemStack {
        unsafe { (host().client_slot)(self.raw, 2, 0) }
    }

    /// The open menu as (kind, window, slot count).
    pub fn menu(&self) -> Option<[u32; 3]> {
        let mut out = [0; 3];
        (unsafe { (host().client_menu)(self.raw, &mut out) } != 0).then_some(out)
    }

    /// Entities the client knows, the local player left out.
    pub fn entities(&self) -> Vec<u64> {
        let mut buf = vec![0u64; 256];
        let n = unsafe { (host().client_entities)(self.raw, buf.as_mut_ptr(), buf.len()) };
        if n > buf.len() {
            buf.resize(n, 0);
            unsafe { (host().client_entities)(self.raw, buf.as_mut_ptr(), buf.len()) };
        }
        buf.truncate(n);
        buf
    }

    pub fn entity(&self, id: u64) -> Option<abi::EntityState> {
        let mut st = abi::EntityState::default();
        (unsafe { (host().client_entity)(self.raw, id, &mut st) } != 0).then_some(st)
    }

    /// Where an element of the top screen is (fractions of the window: x, y, w, h):
    /// `slot:N`, `item:N` (creative list), `entity` (the entity figure), `scrollbar` (the first scroll bar's track), `class:name` (the first part of that theme class), `id:name` or `text:words`.
    pub fn ui_rect(&self, query: &str) -> Option<[f32; 4]> {
        let mut r = [0.0; 4];
        (unsafe { (host().client_ui_rect)(self.raw, s(query), &mut r) } != 0).then_some(r)
    }

    /// Moves the mouse to the middle of a UI element and clicks it with `button`.
    pub fn click_ui(&self, query: &str, button: u32) -> bool {
        let Some([x, y, w, h]) = self.ui_rect(query) else { return false };
        self.cursor(x + w / 2.0, y + h / 2.0);
        self.click(button, 0.0);
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn draw(&self, group: Option<&crate::GizmoGroup>, kind: u32, a: [f64; 3], b: [f64; 3], color: u32, key: &str, args: &[crate::Arg]) {
        let raw = crate::ui::raw_args(args);
        unsafe { (host().client_draw)(self.raw, s(group.map_or("", |g| g.name())), kind, &a, &b, color, s(key), crate::abi::Slice::new(&raw)) }
    }

    /// The server's `draw_*` for this frame only: always, or with a `group` while the player
    /// shows it (`/gizmo <group>`). Colors are 0xRRGGBBAA.
    pub fn draw_line(&self, group: Option<&crate::GizmoGroup>, a: [f64; 3], b: [f64; 3], color: u32) {
        self.draw(group, gk::LINE, a, b, color, "", &[]);
    }

    pub fn draw_box(&self, group: Option<&crate::GizmoGroup>, min: [f64; 3], max: [f64; 3], color: u32) {
        self.draw(group, gk::BOX, min, max, color, "", &[]);
    }

    /// See-through by `color`'s alpha (range previews, blueprints).
    pub fn draw_fill(&self, group: Option<&crate::GizmoGroup>, min: [f64; 3], max: [f64; 3], color: u32) {
        self.draw(group, gk::FILL, min, max, color, "", &[]);
    }

    /// The translation `key` with `args` at a world position, `size` times as big as menu text.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_text(&self, group: Option<&crate::GizmoGroup>, pos: [f64; 3], key: &str, args: &[crate::Arg], color: u32, size: f32) {
        self.draw(group, gk::LABEL, pos, [size as f64, 0.0, 0.0], color, key, args);
    }

    /// The translation `key` with `args`, centered at fractions of the window, `size` times as
    /// big as menu text.
    #[allow(clippy::too_many_arguments)]
    pub fn screen_text(&self, x: f32, y: f32, key: &str, args: &[crate::Arg], color: u32, size: f32) {
        self.draw(None, gk::SCREEN_LABEL, [x as f64, y as f64, 0.0], [size as f64, 0.0, 0.0], color, key, args);
    }

    /// Sends `value` on `channel` to the server.
    pub fn send<T: Payload>(&self, channel: &Channel<T>, value: &T) {
        let data = value.encode();
        unsafe { (host().client_send_packet)(self.raw, s(channel.name()), s(&T::schema()), Slice::new(&data)) }
    }

    /// Runs a command as this player (`/tp ~ ~5 ~`): a client command here, any other on the
    /// server. The answer goes to `command_reply(id)`, not the chat.
    pub fn run_command(&self, line: &str) -> u32 {
        self.command(abi::client_op::RUN_COMMAND, line, 0.0) as u32
    }

    /// The answer of `run_command`'s `id` once it came, its lines in the client's language; read
    /// once.
    pub fn command_reply(&self, id: u32) -> Option<String> {
        let mut out = Str::EMPTY;
        let got = unsafe { (host().client_command_reply)(self.raw, id, &mut out) } != 0;
        got.then(|| unsafe { out.as_str() }.to_string())
    }
}

/// What a channel carries: a `value!` struct or number (checked by its layout like events),
/// `String`, `Vec<u8>` or `()`.
pub trait Payload: Sized + 'static {
    fn schema() -> String;
    fn encode(&self) -> Vec<u8>;
    fn decode(bytes: &[u8]) -> Option<Self>;
}

impl<T: Value> Payload for T {
    fn schema() -> String {
        T::schema()
    }
    fn encode(&self) -> Vec<u8> {
        unsafe { std::slice::from_raw_parts(self as *const T as *const u8, size_of::<T>()) }.to_vec()
    }
    fn decode(bytes: &[u8]) -> Option<T> {
        (bytes.len() == size_of::<T>()).then(|| unsafe { std::ptr::read_unaligned(bytes.as_ptr() as *const T) })
    }
}

impl Payload for String {
    fn schema() -> String {
        "text".into()
    }
    fn encode(&self) -> Vec<u8> {
        self.as_bytes().to_vec()
    }
    fn decode(bytes: &[u8]) -> Option<String> {
        String::from_utf8(bytes.to_vec()).ok()
    }
}

impl Payload for Vec<u8> {
    fn schema() -> String {
        "bytes".into()
    }
    fn encode(&self) -> Vec<u8> {
        self.clone()
    }
    fn decode(bytes: &[u8]) -> Option<Vec<u8>> {
        Some(bytes.to_vec())
    }
}

impl Payload for () {
    fn schema() -> String {
        "unit".into()
    }
    fn encode(&self) -> Vec<u8> {
        Vec::new()
    }
    fn decode(_: &[u8]) -> Option<()> {
        Some(())
    }
}

/// A message channel between a mod's client and server halves, carrying `T`; both ends declare
/// the same type, which the engine checks like event values:
///
/// ```ignore
/// value! { pub struct Boost { pub power: f32 } }
/// static BOOST: Channel<Boost> = Channel::named("boost");
/// r.packet(&BOOST, |p, b| { /* p.player(), b.power */ });   // server
/// client.send(&BOOST, &Boost { power: 2.0 });               // client
/// ```
pub struct Channel<T: Payload> {
    name: &'static str,
    full: std::sync::OnceLock<String>,
    _p: std::marker::PhantomData<fn() -> T>,
}

impl<T: Payload> Channel<T> {
    pub const fn named(name: &'static str) -> Self {
        Channel { name, full: std::sync::OnceLock::new(), _p: std::marker::PhantomData }
    }
    /// For names made at runtime; lives as long as the game.
    pub fn generated(name: String) -> &'static Self {
        Box::leak(Box::new(Channel::named(Box::leak(name.into_boxed_str()))))
    }
    pub fn name(&self) -> &str {
        self.full.get_or_init(|| crate::full_name(self.name))
    }
}

/// RGBA pixels of a screenshot or loaded image: (width, height, pixels).
pub fn image(id: u64) -> Option<(u32, u32, &'static [u8])> {
    let (mut w, mut h) = (0, 0);
    let p = unsafe { (host().image_get)(id, &mut w, &mut h) };
    (!p.is_null()).then(|| (w, h, unsafe { std::slice::from_raw_parts(p, (w * h * 4) as usize) }))
}

pub fn image_load(path: &str) -> Option<u64> {
    let id = unsafe { (host().image_load)(s(path)) };
    (id != 0).then_some(id)
}

pub fn image_save(path: &str, w: u32, h: u32, rgba: &[u8]) -> bool {
    unsafe { (host().image_save)(s(path), Slice::new(rgba), w, h) != 0 }
}

pub fn image_free(id: u64) {
    unsafe { (host().image_free)(id) }
}

/// `--name value` from the command line.
pub fn launch_option(name: &str) -> Option<String> {
    let v = unsafe { (host().launch_option)(s(name)).as_str() };
    (!v.is_empty()).then(|| v.to_string())
}

/// A message on a channel, on the server.
pub struct Packet {
    args: abi::PacketArgs,
}

impl Packet {
    pub fn world(&self) -> World {
        World::from_raw(self.args.world)
    }
    pub fn player(&self) -> u64 {
        self.args.player
    }
    /// Answers the sender on `channel`.
    pub fn reply<R: Payload>(&self, channel: &Channel<R>, value: &R) {
        self.world().send(self.args.player, channel, value);
    }
}

impl World {
    /// Sends `value` on `channel` to `player`'s client.
    pub fn send<T: Payload>(&self, player: u64, channel: &Channel<T>, value: &T) {
        let data = value.encode();
        unsafe { (host().send_packet)(self.raw(), player, s(channel.name()), s(&T::schema()), Slice::new(&data)) }
    }

    /// Opens `menu` on the block entity at `pos`; false (and a logged reason) if there is none.
    pub fn open_menu_at(&self, player: u64, menu: &crate::Menu, pos: [i32; 3]) -> bool {
        self.open_menu_kind_at(player, menu.id(), pos)
    }

    pub fn open_menu_kind_at(&self, player: u64, menu: u32, pos: [i32; 3]) -> bool {
        unsafe { (host().open_menu_at)(self.raw(), player, menu, &pos) != 0 }
    }

    /// An inventory of `slots` not tied to a block (tests, backpacks): menus open on it like on
    /// a chest. Never saved; remove it with `remove`.
    pub fn virtual_inventory(&self, slots: u32, pos: [f64; 3]) -> u64 {
        unsafe { (host().inventory_create)(self.raw(), slots, &pos) }
    }

    /// Moves an entity (players included) at the end of the tick, to `dim` or the same dimension.
    pub fn teleport(&self, id: u64, dim: Option<&crate::Dimension>, pos: [f64; 3]) {
        let d = dim.map_or(u32::MAX, |d| d.id());
        unsafe { (host().entity_teleport)(self.raw(), id, d, &pos) }
    }

    pub fn set_time(&self, time: u64) {
        unsafe { (host().set_time)(self.raw(), time) }
    }

    /// 0 survival, 1 creative, 2 spectator.
    pub fn set_game_mode(&self, player: u64, mode: u32) {
        unsafe { (host().set_game_mode)(self.raw(), player, mode) }
    }
}

impl Registry {
    /// Every frame on the client, before input is handled.
    pub fn client_frame(&mut self, f: impl Fn(&Client) + Send + Sync + 'static) {
        let cb = callback(move |a: &mut abi::ClientFrameArgs| f(&Client::from_raw(a.client)));
        unsafe { (host().reg_client_hook)(self.raw, abi::client_hook::FRAME, cb) }
    }

    /// Cosmetics of entities of `kind` on the client, every frame (wing flaps, sparks, purring),
    /// instead of the engine's steps, idle noises and death smoke; `e.engine_effects()` keeps
    /// those. One per kind: a later mod's replaces an earlier one's.
    pub fn client_entity_tick(&mut self, kind: &crate::Entity, f: impl Fn(&Client, &ClientEntity) + Send + Sync + 'static) {
        let cb = callback(move |a: &mut abi::ClientEntityArgs| f(&Client::from_raw(a.client), unsafe { &*(a as *const abi::ClientEntityArgs as *const ClientEntity) }));
        unsafe { (host().reg_client_entity_tick)(self.raw, s(kind.name()), cb) }
    }

    /// When the top screen changes; the name is empty back in the world.
    pub fn client_screen(&mut self, f: impl Fn(&Client, &str) + Send + Sync + 'static) {
        let cb = callback(move |a: &mut abi::ClientScreenArgs| f(&Client::from_raw(a.client), unsafe { a.screen.as_str() }));
        unsafe { (host().reg_client_hook)(self.raw, abi::client_hook::SCREEN, cb) }
    }

    /// Server side of `channel`: called when a client sends on it.
    /// Server side of a channel: `f` gets every value a client sends on it, with who sent it.
    pub fn packet<T: Payload>(&mut self, channel: &Channel<T>, f: impl Fn(&Packet, T) + Send + Sync + 'static) {
        let name = channel.name();
        let cb = callback(move |a: &mut abi::PacketArgs| {
            if let Some(v) = T::decode(unsafe { a.data.as_slice() }) {
                f(unsafe { &*(a as *const abi::PacketArgs as *const Packet) }, v)
            }
        });
        unsafe { (host().reg_packet)(self.raw, s(name), s(&T::schema()), 0, cb) }
    }

    /// Client side of channel `name`: called when the server sends on it.
    pub fn client_packet<T: Payload>(&mut self, channel: &Channel<T>, f: impl Fn(&Client, T) + Send + Sync + 'static) {
        let name = channel.name();
        let cb = callback(move |a: &mut abi::ClientPacketArgs| {
            if let Some(v) = T::decode(unsafe { a.data.as_slice() }) {
                f(&Client::from_raw(a.client), v)
            }
        });
        unsafe { (host().reg_packet)(self.raw, s(name), s(&T::schema()), 1, cb) }
    }

    /// Key binding with a default key (`J`, `KeyJ`, `F6`, `space`, `lshift`, `mouse3`), listed
    /// in Controls as `key.<ns>.<name>`; what it does with `.on_press(..)` or
    /// `.on_press_server(..)`, or read with `Client::pressed`/`held`. Keys only act while the
    /// world has the input (not while typing in the chat or with a screen open).
    pub fn key<'a>(&'a mut self, key: &'static Key, default: &str) -> KeyBuilder<'a> {
        unsafe { (host().reg_key)(self.raw, s(key.name()), s(default)) }
        KeyBuilder { reg: self, key }
    }
}

pub struct KeyBuilder<'a> {
    reg: &'a mut Registry,
    key: &'static Key,
}

impl KeyBuilder<'_> {
    /// On the client, every time the key goes down.
    pub fn on_press(self, f: impl Fn(&Client) + Send + Sync + 'static) -> Self {
        let key = self.key;
        // By the edge of `held`, so a press is seen whichever frame hook made it.
        let was = std::sync::atomic::AtomicBool::new(false);
        self.reg.client_frame(move |c| {
            let now = c.held(key);
            if now && !was.swap(now, std::sync::atomic::Ordering::Relaxed) {
                f(c)
            } else if !now {
                was.store(false, std::sync::atomic::Ordering::Relaxed);
            }
        });
        self
    }

    /// On the server, for the player who pressed it (`p.player()`, `p.world()`): the engine
    /// carries the press over, no channel of yours needed.
    pub fn on_press_server(self, f: impl Fn(&Packet) + Send + Sync + 'static) -> Self {
        let key = self.key;
        let channel: &'static Channel<()> = Channel::generated(format!("{}.pressed", key.name()));
        self.reg.packet(channel, move |p, ()| f(p));
        KeyBuilder { reg: self.reg, key }.on_press(move |c| c.send(channel, &()))
    }
}

/// The engine's key bindings.
pub mod keys {
    use crate::Key;
    pub static FORWARD: Key = Key::named("core:forward");
    pub static BACK: Key = Key::named("core:back");
    pub static LEFT: Key = Key::named("core:left");
    pub static RIGHT: Key = Key::named("core:right");
    pub static JUMP: Key = Key::named("core:jump");
    pub static SNEAK: Key = Key::named("core:sneak");
    pub static SPRINT: Key = Key::named("core:sprint");
    pub static FLY: Key = Key::named("core:fly");
    pub static INVENTORY: Key = Key::named("core:inventory");
    pub static DROP: Key = Key::named("core:drop");
    pub static COMMAND: Key = Key::named("core:command");
    pub static CHAT: Key = Key::named("core:chat");
    pub static DEBUG: Key = Key::named("core:debug");
    pub static SCREENSHOT: Key = Key::named("core:screenshot");
    pub static PERSPECTIVE: Key = Key::named("core:perspective");
    pub static HIDE_HUD: Key = Key::named("core:hide_hud");
    pub static RELOAD: Key = Key::named("core:reload");
    pub static RECIPES: Key = Key::named("core:recipes");
    pub static USES: Key = Key::named("core:uses");
    pub static ATTACK: Key = Key::named("core:attack");
    pub static USE: Key = Key::named("core:use");
    pub static PICK: Key = Key::named("core:pick");
}

/// The engine's screens: open them, add parts to them, or register one of these handles to
/// replace it.
pub mod screens {
    use crate::Screen;
    pub static TITLE: Screen = Screen::named("core:title");
    pub static PAUSE: Screen = Screen::named("core:pause");
    pub static SETTINGS: Screen = Screen::named("core:settings");
    pub static LANGUAGE: Screen = Screen::named("core:language");
    pub static CONTROLS: Screen = Screen::named("core:controls");
    pub static WORLDS: Screen = Screen::named("core:worlds");
    pub static CREATE_WORLD: Screen = Screen::named("core:create_world");
    pub static CHAT: Screen = Screen::named("core:chat");
    pub static INVENTORY: Screen = Screen::named("core:inventory");
    pub static MOD_SETTINGS: Screen = Screen::named("core:mod_settings");
    pub static DEATH: Screen = Screen::named("core:death");
    pub static MODS: Screen = Screen::named("core:mods");
    pub static PROFILE: Screen = Screen::named("core:profile");
    pub static MULTIPLAYER: Screen = Screen::named("core:multiplayer");
    pub static RESOURCE_PACKS: Screen = Screen::named("core:resource_packs");
    pub static SHADER_PACKS: Screen = Screen::named("core:shader_packs");
    pub static SOUND: Screen = Screen::named("core:sound");
    pub static HUD_EDITOR: Screen = Screen::named("core:hud_editor");
    pub static DEBUG_OPTIONS: Screen = Screen::named("core:debug_options");
}

/// One entity in `Registry::client_entity_tick`.
#[repr(transparent)]
pub struct ClientEntity(abi::ClientEntityArgs);

impl ClientEntity {
    pub fn id(&self) -> u64 {
        self.0.entity
    }
    pub fn kind(&self) -> u32 {
        self.0.kind
    }
    pub fn pos(&self) -> [f64; 3] {
        self.0.pos
    }
    pub fn yaw(&self) -> f32 {
        self.0.yaw
    }
    /// Distance walked so far; steps go by how it grows.
    pub fn walked(&self) -> f32 {
        self.0.walked
    }
    /// Seconds into its death animation (1: over, the last call), `None` while alive.
    pub fn dying(&self) -> Option<f32> {
        (self.0.dying >= 0.0).then_some(self.0.dying)
    }
    /// Seconds since this client first saw it.
    pub fn age(&self) -> f32 {
        self.0.age
    }
    /// Seconds since the last call.
    pub fn dt(&self) -> f32 {
        self.0.dt
    }
    /// What the engine does for kinds without a tick: steps, idle noises, death smoke.
    pub fn engine_effects(&self) {
        let mut a = abi::ClientEntityArgs { ..self.0 };
        unsafe { (host().client_entity_defaults)(&mut a) }
    }
}
