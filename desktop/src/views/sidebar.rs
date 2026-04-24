use gpui::*;
use gpui::{ObjectFit, StyledImage};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use crate::models::channel::Channel;
use crate::views::icons::{IconCache, IconName};

// Embed avatars that aren't on YouTube CDN. Dev-mode bypass : the Node
// server at :4500 only serves `/avatars/*` in production (via
// `express.static(client/dist)`), so in dev these would 404 and the
// sidebar would show a grey placeholder. Embedded bytes sidestep the
// fetch entirely.
const AVATAR_NOOB: &[u8] = include_bytes!("../../assets/noob.jpg");
const AVATAR_PIERRE_CHABRIER: &[u8] = include_bytes!("../../assets/pierre-chabrier.jpg");
const AVATAR_HITS_DU_MOMENT: &[u8] = include_bytes!("../../assets/hits-du-moment.png");

/// Event emitted when user clicks a channel in the sidebar.
#[derive(Clone, Debug)]
pub struct ChannelSelected {
    pub channel_id: String,
    /// Kept for context even though the desktop client no longer needs them
    /// (server is the source of truth for what plays on which channel).
    #[allow(dead_code)]
    pub channel_name: String,
    #[allow(dead_code)]
    pub handle: String,
}

impl EventEmitter<ChannelSelected> for SidebarView {}

/// Event emitted when the hovered channel changes. `None` = the pointer
/// left a channel button (no replacement). The tuple carries
/// `(channel_id, channel_name)` — the id is used by AppView for
/// hover-triggered preload (so the first click on a cold channel is
/// already warm), the name drives the tooltip.
#[derive(Clone, Debug)]
pub struct ChannelHovered(pub Option<(String, String)>);

impl EventEmitter<ChannelHovered> for SidebarView {}

/// Right-click on a channel toggles its favourite state. AppView
/// listens, updates the local Settings (and pushes to the server if
/// the user is logged in), then re-renders the sidebar.
#[derive(Clone, Debug)]
pub struct ChannelFavoriteToggle(pub String);

impl EventEmitter<ChannelFavoriteToggle> for SidebarView {}

pub struct SidebarView {
    pub channels: Vec<Channel>,
    pub selected: usize,
    pub avatars: HashMap<String, Arc<Image>>,
    pub search_query: String,
    /// Channel IDs currently parked in the player's memory cache
    /// (most-recent first). Pushed in via `set_memory_channel_ids`
    /// from AppView whenever the player emits a cache change.
    pub memory_channel_ids: Vec<String>,
    /// Channel IDs the logged-in user has favorited. Set via
    /// `set_favorites`; rendered between Mémoire and TV when
    /// `user_logged_in` is true.
    pub favorite_channel_ids: Vec<String>,
    /// Whether the user is currently logged in. Toggles the
    /// visibility of the favorites section.
    pub user_logged_in: bool,
    /// Per-sidebar icon cache for the section headers.
    icons: IconCache,
}

impl SidebarView {
    pub fn new() -> Self {
        // Empty list until the server responds. The `background_tasks
        // ::channels_and_avatars` poller replaces it via
        // `set_channels_from_server` as soon as `GET /api/tv/channels`
        // returns (typically 100-500 ms after boot). If the server is
        // unreachable, the sidebar stays empty — which is honest :
        // without a server the whole TV pipeline is down anyway.
        //
        // Avatars for local channels (noob, pierre-chabrier,
        // hits-du-moment) are embedded in the binary so the sidebar
        // has artwork available the moment `set_channels_from_server`
        // populates the list — no flash of missing images. Remote
        // avatars (YouTube CDN URLs for the 49 other channels) are
        // fetched by the background task after the channel list
        // arrives.
        let mut avatars = HashMap::new();
        avatars.insert(
            "noob".to_string(),
            Arc::new(Image::from_bytes(ImageFormat::Jpeg, AVATAR_NOOB.to_vec())),
        );
        avatars.insert(
            "pierre-chabrier".to_string(),
            Arc::new(Image::from_bytes(ImageFormat::Jpeg, AVATAR_PIERRE_CHABRIER.to_vec())),
        );
        avatars.insert(
            "hits-du-moment".to_string(),
            Arc::new(Image::from_bytes(ImageFormat::Png, AVATAR_HITS_DU_MOMENT.to_vec())),
        );

        Self {
            channels: Vec::new(),
            selected: 0,
            avatars,
            search_query: String::new(),
            memory_channel_ids: Vec::new(),
            favorite_channel_ids: Vec::new(),
            user_logged_in: false,
            icons: IconCache::new(),
        }
    }

    pub fn set_memory_channel_ids(&mut self, ids: Vec<String>) {
        self.memory_channel_ids = ids;
    }

    pub fn set_favorites(&mut self, ids: Vec<String>) {
        self.favorite_channel_ids = ids;
    }

    pub fn set_logged_in(&mut self, logged_in: bool) {
        self.user_logged_in = logged_in;
    }

    /// Return the (id, name, handle) of the currently-selected channel.
    pub fn selected_channel(&self) -> Option<(String, String, String)> {
        self.channels.get(self.selected).map(|c| {
            (c.id.clone(), c.name.clone(), c.handle.clone())
        })
    }

    pub fn set_search_query(&mut self, q: String) {
        self.search_query = q;
    }

    /// Tuple per channel : `(id, name, handle, avatar_url)`. The server
    /// is the single source of truth — empty handle/avatar fields are
    /// accepted as-is ; no client-side fallback list to lie about
    /// missing metadata. Channels without an avatar_url render with
    /// the first letter of their name as placeholder (the sidebar
    /// render path already handles that).
    pub fn set_channels_from_server(
        &mut self,
        server: Vec<(String, String, String, String)>,
    ) {
        let mut new_channels: Vec<Channel> = server
            .into_iter()
            .map(|(id, name, handle, avatar)| Channel {
                id,
                name,
                handle,
                avatar_url: avatar,
            })
            .collect();
        new_channels.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

        // Prune avatars for channels no longer present (prevents leak on channel churn)
        let valid_ids: std::collections::HashSet<&str> =
            new_channels.iter().map(|c| c.id.as_str()).collect();
        self.avatars.retain(|k, _| valid_ids.contains(k.as_str()));

        // Preserve the currently-selected channel across the remap. If
        // the current id is still present → keep it. Otherwise pick a
        // fresh random so the user doesn't get silently bumped to
        // channel 0.
        let current_id = self
            .channels
            .get(self.selected)
            .map(|c| c.id.clone());
        self.channels = new_channels;
        if let Some(id) = current_id {
            if let Some(ix) = self.channels.iter().position(|c| c.id == id) {
                self.selected = ix;
                return;
            }
        }
        self.selected = if self.channels.is_empty() {
            0
        } else {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos() as usize)
                .unwrap_or(0)
                % self.channels.len()
        };
    }

    pub fn set_avatar(&mut self, channel_id: String, image: Arc<Image>) {
        self.avatars.insert(channel_id, image);
    }
}

impl Render for SidebarView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.selected;
        let q = self.search_query.to_lowercase();
        let visible: Vec<(usize, Channel)> = self
            .channels
            .iter()
            .enumerate()
            .filter(|(_, c)| q.is_empty() || c.name.to_lowercase().contains(&q))
            .map(|(i, c)| (i, c.clone()))
            .collect();

        // Resolve memory channel IDs to full Channel records. The
        // CURRENT channel sits at the top of "Mémoire" — the next 4
        // are the LRU-cached previously-watched channels.
        let mut memory_visible: Vec<(usize, Channel)> = Vec::new();
        if let Some(cur) = self.channels.get(selected).cloned() {
            memory_visible.push((selected, cur));
        }
        for id in &self.memory_channel_ids {
            // Don't duplicate the current channel if it's also in the
            // LRU history.
            if let Some((i, c)) = self
                .channels
                .iter()
                .enumerate()
                .find(|(_, c)| &c.id == id)
            {
                if i != selected {
                    memory_visible.push((i, c.clone()));
                }
            }
            if memory_visible.len() >= 5 {
                break;
            }
        }

        // Favourites — visible for everyone now (was gated on
        // login). Persisted locally for anonymous users, locally +
        // server-side for logged-in users.
        let favorites_visible: Vec<(usize, Channel)> = self
            .favorite_channel_ids
            .iter()
            .filter_map(|id| {
                self.channels
                    .iter()
                    .enumerate()
                    .find(|(_, c)| &c.id == id)
                    .map(|(i, c)| (i, c.clone()))
            })
            .collect();

        // Build the rendered list: memory + favorites + main list.
        let mut combined: Vec<(usize, Channel)> = Vec::new();
        let memory_count = memory_visible.len();
        let favorites_count = favorites_visible.len();
        let show_favorites = !favorites_visible.is_empty();
        let _ = self.user_logged_in;
        combined.extend(memory_visible);
        combined.extend(favorites_visible);
        combined.extend(visible);

        div()
            .id("channel-sidebar")
            .flex()
            .flex_col()
            .items_center()
            .w(px(56.0))
            .h_full()
            .bg(rgb(0x0e0e10))
            .border_r_1()
            .border_color(rgb(0x2d2d30))
            .overflow_y_scroll()
            .py(px(10.0))
            .gap(px(6.0))
            .children({
                let history_icon = self.icons.get(IconName::History, 14, 0x9b59b6);
                let tv_icon = self.icons.get(IconName::Tv, 14, 0x9b59b6);
                let star_icon = self.icons.get(IconName::Star, 14, 0x9b59b6);
                let favorites_start = memory_count;
                let favorites_end = memory_count + favorites_count;
                let main_start = favorites_end;
                let mut out: Vec<gpui::AnyElement> = Vec::new();

                // ── Mémoire header (if any memory entries) ────
                if memory_count > 0 {
                    out.push(section_header_icon(history_icon.clone()).into_any_element());
                }

                for (pos, (i, ch)) in combined.iter().enumerate() {
                    // Insert section headers BEFORE the relevant tile.
                    if pos == favorites_start && show_favorites && favorites_count > 0 {
                        out.push(section_header_icon(star_icon.clone()).into_any_element());
                    }
                    if pos == main_start {
                        out.push(section_header_icon(tv_icon.clone()).into_any_element());
                    }
                    let suffix = if pos < favorites_start {
                        "mem"
                    } else if pos < main_start {
                        "fav"
                    } else {
                        "main"
                    };
                    out.push(self.render_channel_button(*i, ch.clone(), selected, suffix, cx));
                }
                out
            })
    }
}

/// Small icon-only section header centered in the 56px-wide sidebar.
/// Identifies the Mémoire (history icon) and TV (tv icon) sections
/// purely visually — no text.
fn section_header_icon(icon: Option<Arc<Image>>) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(2.0))
        .my(px(2.0))
        .child(match icon {
            Some(img_handle) => img(img_handle)
                .w(px(14.0))
                .h(px(14.0))
                .into_any_element(),
            None => div().w(px(14.0)).h(px(14.0)).into_any_element(),
        })
        .child(
            div()
                .w(px(24.0))
                .h(px(1.0))
                .bg(rgb(0x2d2d30)),
        )
}

impl SidebarView {
    /// Render one round avatar button. `i` is the channel's index in
    /// `self.channels` (used for selection state); `id_suffix` makes
    /// the GPUI ElementId unique even when the same channel appears
    /// twice (in the Mémoire section AND in the main list).
    fn render_channel_button(
        &self,
        i: usize,
        ch: Channel,
        selected: usize,
        id_suffix: &str,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let is_selected = i == selected;
        let name_initial = ch.name.chars().next().unwrap_or('?').to_string();
        let ch_id = ch.id.clone();
        let ch_name = ch.name.clone();
        let ch_handle = ch.handle.clone();
        let avatar = self.avatars.get(&ch.id).cloned();
        let hover_id = ch.id.clone();
        let hover_name = ch.name.clone();
        let elem_id =
            ElementId::Name(format!("{}-{}", ch.id, id_suffix).into());

        let button = div()
            .id(elem_id)
            .flex_none()
            .w(px(40.0))
            .h(px(40.0))
            .rounded_full()
            .cursor_pointer()
            .border_2()
            .border_color(if is_selected { rgb(0x9b59b6) } else { rgba(0x00000000) })
            .hover(|this| this.border_color(rgb(0x9b59b6)))
            .bg(rgb(0x18181b))
            .overflow_hidden()
            .flex()
            .items_center()
            .justify_center()
            .on_hover(cx.listener(move |_view, hovered: &bool, _window, cx| {
                if *hovered {
                    cx.emit(ChannelHovered(Some((
                        hover_id.clone(),
                        hover_name.clone(),
                    ))));
                } else {
                    cx.emit(ChannelHovered(None));
                }
            }))
            .on_click(cx.listener(move |view, _ev: &ClickEvent, _window, cx| {
                view.selected = i;
                cx.emit(ChannelSelected {
                    channel_id: ch_id.clone(),
                    channel_name: ch_name.clone(),
                    handle: ch_handle.clone(),
                });
                cx.notify();
            }))
            // Right-click toggles favourite. AppView listens, updates
            // settings + persists.
            .on_mouse_down(MouseButton::Right, cx.listener({
                let id = ch.id.clone();
                move |_view: &mut SidebarView, _ev: &MouseDownEvent, _window, cx| {
                    cx.emit(ChannelFavoriteToggle(id.clone()));
                }
            }));

        if let Some(img_handle) = avatar {
            let anim_id: ElementId =
                ElementId::Name(format!("avatar-fade-{}-{}", ch.id, id_suffix).into());
            button
                .child(
                    img(img_handle)
                        .object_fit(ObjectFit::Cover)
                        .flex_none()
                        .w(px(36.0))
                        .h(px(36.0))
                        .rounded_full()
                        .with_animation(
                            anim_id,
                            Animation::new(Duration::from_millis(220))
                                .with_easing(|t| 1.0 - (1.0 - t).powi(3)),
                            |this, t| this.opacity(t),
                        ),
                )
                .into_any_element()
        } else {
            button
                .text_xs()
                .text_color(if is_selected { rgb(0xffffff) } else { rgb(0xaaaaaa) })
                .child(name_initial)
                .into_any_element()
        }
    }
}
