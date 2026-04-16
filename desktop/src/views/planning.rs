//! In-app TV-guide planning view. Shows the schedule for a channel on a
//! weekly 7-column grid (Lun→Dim) with hour rows. No browser, no web —
//! it all runs in GPUI + libmpv-adjacent native rendering.
//!
//! Auto-scrolls so the red "now" line sits centred on open.

use gpui::*;
use gpui_component::input::{InputState, Input, InputEvent};
use std::sync::Arc;

use crate::services::api::{self, PlaylistInfo, ServerChannel};
use crate::views::icons::{IconCache, IconName};

pub const HOUR_PX: f32 = 120.0;
pub const HOURS_VISIBLE: u32 = 24;
pub const GAP_PX: f32 = 4.0;
pub const HEADER_PX: f32 = 72.0;
pub const MIN_BLOCK_FOR_TITLE_PX: f32 = 34.0;

#[derive(Clone, Debug)]
pub struct PlanningClose;
impl EventEmitter<PlanningClose> for PlanningView {}

pub struct PlanningView {
    channels: Vec<ServerChannel>,
    selected_channel_id: String,
    playlist: Option<PlaylistInfo>,
    loading: bool,
    week_offset: i32,
    icons: IconCache,
    scroll_handle: ScrollHandle,
    scrolled_once: bool,
    now_ms: i64,
    #[allow(dead_code)]
    refresh_timer: Option<Task<()>>,
    channel_dropdown_open: bool,
    #[allow(dead_code)]
    channel_select: Entity<InputState>,
    #[allow(dead_code)]
    _subs: Vec<Subscription>,
}

#[derive(Clone, Debug)]
struct Block {
    start_ms: i64,
    end_ms: i64,
    title: String,
    video_id: String,
}

fn start_of_week_ms(now: i64) -> i64 {
    // Monday of the local week at 00:00.
    let secs = now / 1000;
    let t: libc::time_t = secs as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm); }
    // Zero the hour/min/sec
    tm.tm_hour = 0; tm.tm_min = 0; tm.tm_sec = 0;
    // Shift to Monday: tm_wday: 0=Sun, 1=Mon ... 6=Sat
    let dow_from_monday = ((tm.tm_wday + 6) % 7) as i32;
    let day_of_week_start_secs = unsafe { libc::mktime(&mut tm) as i64 };
    (day_of_week_start_secs - dow_from_monday as i64 * 86400) * 1000
}

fn add_days_ms(ms: i64, days: i64) -> i64 {
    ms + days * 86_400_000
}

fn local_time_hhmm(ms: i64) -> String {
    let secs = ms / 1000;
    let t: libc::time_t = secs as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm); }
    format!("{:02}:{:02}", tm.tm_hour, tm.tm_min)
}

fn fmt_day(ms: i64) -> String {
    let secs = ms / 1000;
    let t: libc::time_t = secs as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm); }
    let months = [
        "janv.", "févr.", "mars", "avr.", "mai", "juin",
        "juil.", "août", "sept.", "oct.", "nov.", "déc.",
    ];
    format!("{:02} {}", tm.tm_mday, months[tm.tm_mon as usize])
}

fn is_today_ms(ms: i64, now_ms: i64) -> bool {
    let a = |m: i64| {
        let s = m / 1000;
        let t: libc::time_t = s as libc::time_t;
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        unsafe { libc::localtime_r(&t, &mut tm); }
        (tm.tm_year, tm.tm_mon, tm.tm_mday)
    };
    a(ms) == a(now_ms)
}

fn schedule_between(pl: &PlaylistInfo, from_ms: i64, to_ms: i64) -> Vec<Block> {
    if pl.videos.is_empty() || pl.total_duration <= 0.0 {
        return Vec::new();
    }
    let tv_started_at = pl.tv_started_at as i64;
    let total = pl.total_duration as i64;
    let from_sec = (from_ms - tv_started_at) / 1000;
    let to_sec = (to_ms - tv_started_at) / 1000;
    let mut elapsed = ((from_sec % total) + total) % total;
    let mut acc: i64 = 0;
    let mut idx: usize = 0;
    while idx < pl.videos.len()
        && acc + pl.videos[idx].duration as i64 <= elapsed
    {
        acc += pl.videos[idx].duration as i64;
        idx += 1;
    }
    let inner_offset = elapsed - acc;
    let mut out = Vec::new();
    let mut cur = from_sec;
    let mut first = true;
    while cur < to_sec {
        let v = &pl.videos[idx % pl.videos.len()];
        let remaining = v.duration as i64 - if first { inner_offset } else { 0 };
        first = false;
        let end = (cur + remaining).min(to_sec);
        out.push(Block {
            start_ms: tv_started_at + cur * 1000,
            end_ms: tv_started_at + end * 1000,
            title: v.title.clone(),
            video_id: v.video_id.clone(),
        });
        cur = end;
        idx += 1;
    }
    let _ = elapsed;
    out
}

impl PlanningView {
    pub fn new(
        channels: Vec<ServerChannel>,
        initial_channel: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let channel_select = cx.new(|cx| InputState::new(window, cx));

        // Refresh "now" every 30s so the red line moves.
        let entity = cx.entity().downgrade();
        let refresh_timer = cx.spawn(async move |_, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(30))
                    .await;
                if let Some(e) = entity.upgrade() {
                    let _ = cx.update_entity(&e, |this, cx| {
                        this.now_ms = now_ms();
                        cx.notify();
                    });
                } else {
                    break;
                }
            }
        });

        let mut v = Self {
            channels,
            selected_channel_id: initial_channel,
            playlist: None,
            loading: true,
            week_offset: 0,
            icons: IconCache::new(),
            scroll_handle: ScrollHandle::new(),
            scrolled_once: false,
            now_ms: now_ms(),
            refresh_timer: Some(refresh_timer),
            channel_dropdown_open: false,
            channel_select,
            _subs: Vec::new(),
        };
        v.start_fetch(cx);
        v
    }

    fn start_fetch(&mut self, cx: &mut Context<Self>) {
        self.loading = true;
        self.playlist = None;
        self.scrolled_once = false;
        let ch = self.selected_channel_id.clone();
        let entity = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            let (tx, rx) = std::sync::mpsc::channel::<Option<PlaylistInfo>>();
            std::thread::spawn(move || {
                let _ = tx.send(api::fetch_playlist(&ch).ok());
            });
            for _ in 0..150 {
                if let Ok(pl) = rx.try_recv() {
                    if let Some(e) = entity.upgrade() {
                        let _ = cx.update_entity(&e, |this, cx| {
                            this.playlist = pl;
                            this.loading = false;
                            cx.notify();
                        });
                    }
                    return;
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(100))
                    .await;
            }
            if let Some(e) = entity.upgrade() {
                let _ = cx.update_entity(&e, |this, cx| {
                    this.loading = false;
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn set_channel(&mut self, id: String, cx: &mut Context<Self>) {
        if id == self.selected_channel_id { return; }
        self.selected_channel_id = id;
        self.start_fetch(cx);
    }

    fn set_week(&mut self, offset: i32, cx: &mut Context<Self>) {
        self.week_offset = offset.clamp(0, 1);
        self.scrolled_once = false;
        cx.notify();
    }
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn week_days(week_start_ms: i64) -> [i64; 7] {
    [
        add_days_ms(week_start_ms, 0),
        add_days_ms(week_start_ms, 1),
        add_days_ms(week_start_ms, 2),
        add_days_ms(week_start_ms, 3),
        add_days_ms(week_start_ms, 4),
        add_days_ms(week_start_ms, 5),
        add_days_ms(week_start_ms, 6),
    ]
}

impl Render for PlanningView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now_base_week = start_of_week_ms(self.now_ms);
        let week_start = add_days_ms(now_base_week, self.week_offset as i64 * 7);
        let days = week_days(week_start);
        let week_label = if self.week_offset == 0 { "Cette semaine" } else { "Semaine prochaine" };
        let range_label = format!("{} — {}", fmt_day(days[0]), fmt_day(days[6]));

        // Auto-scroll to the current hour once the grid is laid out. The
        // scrollable below has 25 in-flow "anchor" children (1 header
        // spacer + 24 hour spacers) so `scroll_to_item(hourIdx + 1)`
        // lands exactly on the target hour.
        if !self.scrolled_once {
            let hour = {
                let secs = self.now_ms / 1000;
                let t: libc::time_t = secs as libc::time_t;
                let mut tm: libc::tm = unsafe { std::mem::zeroed() };
                unsafe { libc::localtime_r(&t, &mut tm); }
                tm.tm_hour as usize
            };
            // +1 because anchor 0 is the header spacer. Pick the previous
            // hour so the current hour sits 1 row down from the top
            // (gives a bit of lead-in context like the web does).
            let target = hour.saturating_sub(1).min(HOURS_VISIBLE as usize - 1) + 1;
            self.scroll_handle.scroll_to_item(target);
            self.scrolled_once = true;
        }

        let header = div()
            .flex()
            .items_center()
            .gap(px(16.0))
            .px(px(20.0))
            .py(px(12.0))
            .bg(rgb(0x0d0d10))
            .border_b_1()
            .border_color(rgb(0x1f1f23))
            .child({
                let back_icon = self.icons.get(IconName::Play, 14, 0xefeff1);
                let mut btn = div()
                    .id("plan-back")
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .px(px(12.0))
                    .py(px(6.0))
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(rgb(0x2d2d30))
                    .cursor_pointer()
                    .hover(|this| this.bg(rgb(0x17171a)))
                    .text_xs()
                    .text_color(rgb(0xefeff1))
                    .on_click(cx.listener(|_this, _ev: &ClickEvent, _, cx| {
                        cx.emit(PlanningClose);
                    }));
                if let Some(ic) = back_icon {
                    btn = btn.child(img(ic).w(px(14.0)).h(px(14.0)));
                }
                btn.child("Retour")
            })
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(0xf4f4f6))
                    .child("Programme"),
            )
            .child({
                // Dropdown button — shows the selected channel + chevron,
                // click toggles the popup list of all channels.
                let open = self.channel_dropdown_open;
                div()
                    .id("plan-channel-dropdown")
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .text_xs()
                    .px(px(10.0))
                    .py(px(6.0))
                    .rounded(px(6.0))
                    .bg(rgb(0x17171a))
                    .border_1()
                    .border_color(if open { rgb(0x9b59b6) } else { rgb(0x2d2d30) })
                    .text_color(rgb(0xefeff1))
                    .cursor_pointer()
                    .hover(|this| this.bg(rgb(0x1e1e22)))
                    .child(self.channel_name())
                    .child(div().text_color(rgb(0x8a8a90)).child("▾"))
                    .on_click(cx.listener(|this: &mut PlanningView, _ev: &ClickEvent, _, cx| {
                        this.channel_dropdown_open = !this.channel_dropdown_open;
                        cx.notify();
                    }))
            })
            .child(
                div()
                    .flex_1()
                    .flex()
                    .justify_end()
                    .gap(px(8.0))
                    .child(
                        div()
                            .id("plan-week-prev")
                            .w(px(28.0))
                            .h(px(28.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(6.0))
                            .border_1()
                            .border_color(rgb(0x2d2d30))
                            .cursor_pointer()
                            .text_color(rgb(0xd0d0d4))
                            .hover(|this| this.bg(rgb(0x17171a)))
                            .child("◂")
                            .on_click(cx.listener(|this, _ev: &ClickEvent, _, cx| {
                                if this.week_offset > 0 {
                                    this.set_week(this.week_offset - 1, cx);
                                }
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .min_w(px(180.0))
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(0xe8e8ea))
                                    .child(week_label),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(0x8a8a90))
                                    .child(range_label),
                            ),
                    )
                    .child(
                        div()
                            .id("plan-week-next")
                            .w(px(28.0))
                            .h(px(28.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(6.0))
                            .border_1()
                            .border_color(rgb(0x2d2d30))
                            .cursor_pointer()
                            .text_color(rgb(0xd0d0d4))
                            .hover(|this| this.bg(rgb(0x17171a)))
                            .child("▸")
                            .on_click(cx.listener(|this, _ev: &ClickEvent, _, cx| {
                                if this.week_offset < 1 {
                                    this.set_week(this.week_offset + 1, cx);
                                }
                            })),
                    ),
            );

        let body = if self.loading {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(0x8a8a90))
                .child("Chargement du planning…")
                .into_any_element()
        } else if let Some(pl) = self.playlist.clone() {
            self.render_grid(pl, days, cx).into_any_element()
        } else {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(0xef4444))
                .child("Playlist indisponible.")
                .into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x0a0a0d))
            .text_color(rgb(0xefeff1))
            .child(header)
            .child(body)
            .child(self.render_channel_dropdown(cx))
    }
}

impl PlanningView {
    fn channel_name(&self) -> String {
        self.channels
            .iter()
            .find(|c| c.id == self.selected_channel_id)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| self.selected_channel_id.clone())
    }

    fn render_channel_dropdown(&self, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.channel_dropdown_open {
            return div().into_any_element();
        }
        let items = self
            .channels
            .iter()
            .cloned()
            .map(|ch| {
                let selected = ch.id == self.selected_channel_id;
                let id = ch.id.clone();
                div()
                    .id(SharedString::from(format!("dd-{}", ch.id)))
                    .px(px(12.0))
                    .py(px(7.0))
                    .text_xs()
                    .cursor_pointer()
                    .bg(if selected { rgb(0x2d1e3a) } else { rgb(0x131316) })
                    .text_color(if selected { rgb(0xefddff) } else { rgb(0xd0d0d4) })
                    .border_b_1()
                    .border_color(rgb(0x1f1f23))
                    .hover(|this| this.bg(rgb(0x1f1f23)).text_color(rgb(0xffffff)))
                    .on_click(cx.listener(move |this: &mut PlanningView, _ev: &ClickEvent, _, cx| {
                        this.set_channel(id.clone(), cx);
                        this.channel_dropdown_open = false;
                        cx.notify();
                    }))
                    .child(ch.name)
            })
            .collect::<Vec<_>>();

        deferred(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .on_mouse_down(MouseButton::Left, cx.listener(|this: &mut PlanningView, _ev: &MouseDownEvent, _, cx| {
                    // Click-away to close.
                    this.channel_dropdown_open = false;
                    cx.notify();
                }))
                .child(
                    div()
                        // Popup panel positioned below the dropdown button
                        // in the header (roughly — absolute coords).
                        .absolute()
                        .top(px(56.0))
                        .left(px(220.0))
                        .w(px(260.0))
                        .max_h(px(420.0))
                        .rounded(px(8.0))
                        .overflow_hidden()
                        .bg(rgb(0x131316))
                        .border_1()
                        .border_color(rgb(0x2d2d30))
                        .shadow_lg()
                        .on_mouse_down(MouseButton::Left, |_, _, _| {
                            // Swallow clicks inside the panel so the outer
                            // click-away handler doesn't fire.
                        })
                        .child(
                            div()
                                .id("plan-dd-scroll")
                                .overflow_y_scroll()
                                .max_h(px(420.0))
                                .children(items),
                        )
                        .occlude(),
                ),
        )
        .with_priority(20)
        .into_any_element()
    }

    fn render_grid(
        &self,
        pl: PlaylistInfo,
        days: [i64; 7],
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let now_ms = self.now_ms;
        let day_ms = 86_400_000i64;
        let total_height = HEADER_PX + HOUR_PX * HOURS_VISIBLE as f32;

        // Hour ruler column
        let hours_col = {
            let mut col = div()
                .w(px(64.0))
                .h(px(total_height))
                .flex_shrink_0()
                .bg(rgb(0x0a0a0d))
                .border_r_1()
                .border_color(rgb(0x1f1f23));
            // Spacer
            col = col.child(div().h(px(HEADER_PX)).bg(rgb(0x0a0a0d)));
            for h in 0..HOURS_VISIBLE {
                col = col.child(
                    div()
                        .h(px(HOUR_PX))
                        .flex()
                        .justify_end()
                        .pr(px(8.0))
                        .text_xs()
                        .text_color(rgb(0x5a5a62))
                        .child(format!("{:02}:00", h)),
                );
            }
            col
        };

        // Day columns
        let day_cols: Vec<AnyElement> = days
            .iter()
            .enumerate()
            .map(|(i, &day_start)| {
                let is_today = is_today_ms(day_start, now_ms);
                let day_end = day_start + day_ms;
                let blocks = schedule_between(&pl, day_start, day_end);

                let mut col = div()
                    .flex_1()
                    .h(px(total_height))
                    .flex()
                    .flex_col()
                    .border_r_1()
                    .border_color(rgb(0x17171a));
                if is_today {
                    col = col.bg(rgba(0x9b59b60a));
                }

                // Header
                let mut header = div()
                    .h(px(HEADER_PX))
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .border_b_1()
                    .border_color(rgb(0x1f1f23))
                    .bg(rgb(0x0f0f13));
                if is_today {
                    header = header.bg(rgb(0x3a2847));
                }
                header = header
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .text_color(if is_today { rgb(0xefddff) } else { rgb(0xc8c8cc) })
                            .child(
                                ["LUN", "MAR", "MER", "JEU", "VEN", "SAM", "DIM"][i].to_string(),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x8a8a90))
                            .child(fmt_day(day_start)),
                    );
                if is_today {
                    header = header.child(
                        div()
                            .mt(px(2.0))
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .px(px(6.0))
                            .py(px(1.0))
                            .rounded(px(999.0))
                            .bg(rgb(0x9b59b6))
                            .text_color(rgb(0xffffff))
                            .child("AUJOURD'HUI"),
                    );
                }

                let column_body_h = HOUR_PX * HOURS_VISIBLE as f32;
                let mut body = div()
                    .w_full()
                    .h(px(column_body_h))
                    .relative()
                    .overflow_hidden();

                for b in &blocks {
                    let raw_top = ((b.start_ms - day_start) as f32 / 1000.0 / 3600.0) * HOUR_PX;
                    let raw_len = ((b.end_ms - b.start_ms) as f32 / 1000.0 / 3600.0) * HOUR_PX;
                    let top = raw_top + GAP_PX / 2.0;
                    let height = raw_len - GAP_PX;
                    if height < 2.0 {
                        continue;
                    }
                    let is_current = is_today
                        && b.start_ms <= now_ms
                        && b.end_ms > now_ms;
                    let block_bg = if is_current { rgb(0xdc2626) } else { rgb(0x7c3f99) };
                    let mut block = div()
                        .absolute()
                        .left(px(4.0))
                        .right(px(4.0))
                        .top(px(top))
                        .h(px(height))
                        .rounded(px(6.0))
                        .bg(block_bg)
                        .px(px(8.0))
                        .py(px(4.0))
                        .text_xs()
                        .text_color(rgb(0xffffff))
                        .flex()
                        .flex_col()
                        .gap(px(1.0))
                        .overflow_hidden()
                        .border_1()
                        .border_color(rgba(0xffffff18));
                    block = block.child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgba(0xffffffd9))
                            .child(local_time_hhmm(b.start_ms)),
                    );
                    if height >= MIN_BLOCK_FOR_TITLE_PX {
                        block = block.child(
                            div()
                                .text_xs()
                                .text_color(rgb(0xffffff))
                                .child(b.title.clone()),
                        );
                    }
                    if is_current {
                        block = block.child(
                            div()
                                .mt(px(2.0))
                                .text_xs()
                                .font_weight(FontWeight::BOLD)
                                .child("● EN DIRECT"),
                        );
                    }
                    body = body.child(block);
                }

                col = col.child(header).child(body);
                col.into_any_element()
            })
            .collect();

        let _ = cx;

        // Determine if "now" falls inside the displayed week — if yes,
        // render a red line overlay that spans ALL seven day columns.
        let any_today = days.iter().any(|&d| is_today_ms(d, now_ms));
        let now_y = if any_today {
            let secs_of_day = {
                let t: libc::time_t = (now_ms / 1000) as libc::time_t;
                let mut tm: libc::tm = unsafe { std::mem::zeroed() };
                unsafe { libc::localtime_r(&t, &mut tm); }
                tm.tm_hour as f32 * 3600.0 + tm.tm_min as f32 * 60.0 + tm.tm_sec as f32
            };
            Some(HEADER_PX + (secs_of_day / 3600.0) * HOUR_PX)
        } else {
            None
        };

        // Scrollable layout trick: 25 flow anchor children (header spacer
        // + 24 hour spacers, all 0-opacity) give scroll_to_item() real
        // target bounds. The actual grid + now-line sit as absolute
        // overlays on top of them.
        let mut scroll = div()
            .id("plan-scroll")
            .flex_1()
            .overflow_y_scroll()
            .relative()
            .track_scroll(&self.scroll_handle);
        scroll = scroll.child(div().h(px(HEADER_PX)));
        for _ in 0..HOURS_VISIBLE {
            scroll = scroll.child(div().h(px(HOUR_PX)));
        }
        scroll = scroll.child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(px(total_height))
                .flex()
                .child(hours_col)
                .children(day_cols),
        );
        if let Some(y) = now_y {
            scroll = scroll.child(
                div()
                    .absolute()
                    .top(px(y))
                    .left_0()
                    .right_0()
                    .h(px(2.0))
                    .bg(rgb(0xef4444))
                    .child(
                        div()
                            .absolute()
                            .left(px(6.0))
                            .top(px(-10.0))
                            .px(px(6.0))
                            .py(px(1.0))
                            .rounded(px(3.0))
                            .bg(rgb(0xef4444))
                            .text_color(rgb(0xffffff))
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .child(local_time_hhmm(now_ms)),
                    ),
            );
        }
        scroll
    }
}
