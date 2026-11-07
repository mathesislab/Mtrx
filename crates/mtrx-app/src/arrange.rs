//! Arranging the room list: the user's categories, favourites and low
//! priority, and the order of its sections, by dragging rooms between
//! sections and sections to other places, or through menus.

use std::sync::Arc;

use iced::{Point, Subscription, Task, event, mouse};
use mtrx_core::{
    AtHome, Grouping, OwnedRoomId, RoomCategories, RoomSummary, RoomTag, Section, Session, Sort,
};

use crate::{
    app::{self, Popup},
    task,
};

/// The field naming a category.
pub const NAME_INPUT: &str = "category-name";
/// How far the cursor moves before a press on a room becomes a drag.
const DRAG_THRESHOLD: f32 = 5.0;

#[derive(Debug, Clone)]
pub enum Message {
    Categories(RoomCategories),
    /// The cursor entered (`true`) or left a room row.
    Row(OwnedRoomId, bool),
    /// The cursor entered or left a section's grip, which drags it.
    Grip(Section, bool),
    /// The cursor entered or left a section's heading or its row in the
    /// Show menu, which show its grip.
    SectionHovered(Section, bool),
    /// The cursor entered or left the pinned Notes row, which is not dragged.
    NotesRow(bool),
    /// The left mouse button, anywhere.
    Pressed,
    Released,
    Moved(Point),
    /// While dragging, the cursor entered (`true`) or left a section, which
    /// a room would drop into, or whose place a section would take.
    Target(Section, bool),
    ToggleCollapsed(Section),
    /// Collapse (`true`) or expand these sections at once.
    Collapse(Vec<(Section, bool)>),
    /// Open the menu of which sections a space shows expanded.
    ShowMenu,
    RoomMenu(OwnedRoomId),
    CategoryMenu(String),
    MoveTo(OwnedRoomId, Section),
    /// Favourite (the star), low priority or neither, keeping the room's
    /// category.
    Tag(OwnedRoomId, RoomTag),
    /// Create or change a category, from its dialog.
    SaveCategory(CategoryEdit),
    Rename(String),
    Name(String),
    SaveName,
    Delete(String),
    /// Move a section to the place of another.
    MoveSection(Section, Section),
    Sort(String, Sort),
    /// Which rooms of spaces Home lists too, from the settings.
    AtHome(AtHome),
    /// The Notes to self at the top, from the settings.
    NotesAtTop(bool),
    Saved(Result<(), String>),
}

/// What a drag moves: a room into a section, or a section to the place of
/// another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dragged {
    Room(OwnedRoomId),
    Section(Section),
}

/// A room or section pressed, and dragged once the cursor moved far enough.
pub struct Drag {
    pub item: Dragged,
    start: Option<Point>,
    pub at: Option<Point>,
    pub active: bool,
}

/// A category being renamed.
pub struct Naming {
    pub category: String,
    pub name: String,
}

/// A new category, or a changed one (`category`), of `space` (Home for
/// `None`): its name and rooms.
#[derive(Debug, Clone)]
pub struct CategoryEdit {
    pub space: Option<OwnedRoomId>,
    pub category: Option<String>,
    pub name: String,
    pub rooms: Vec<OwnedRoomId>,
}

impl CategoryEdit {
    /// Make the change in `categories`.
    fn apply(&self, categories: &mut RoomCategories) {
        let layout = categories.layout_mut(self.space.as_deref());
        let id = match &self.category {
            Some(id) => {
                let Some(category) = layout.category_mut(id) else {
                    return;
                };
                category.name = self.name.clone();
                // Rooms no longer in it go back to their default section.
                let gone: Vec<_> = (category.rooms.iter())
                    .filter(|r| !self.rooms.contains(r))
                    .cloned()
                    .collect();
                for room in gone {
                    layout.place(&room, None);
                }
                id.clone()
            }
            None => layout.add_category(&self.name),
        };
        for room in &self.rooms {
            // Those already in it keep their place.
            if layout.category_of(room).map(|c| &c.id) != Some(&id) {
                layout.place(room, Some(&id));
            }
        }
    }
}

#[derive(Default)]
pub struct Arrange {
    pub categories: RoomCategories,
    /// The room row or section grip under the cursor.
    pub hovered: Option<Dragged>,
    /// The section whose grip shows.
    pub section_hovered: Option<Section>,
    pub notes_hovered: bool,
    pub drag: Option<Drag>,
    /// The section a drag would drop into.
    pub target: Option<Section>,
    pub naming: Option<Naming>,
}

/// What [`Arrange::update`] works on.
pub struct Context<'a> {
    pub session: &'a Arc<Session>,
    pub space: Option<&'a OwnedRoomId>,
    pub rooms: &'a [RoomSummary],
    pub grouping: Grouping,
    /// The window's popup, where the room list's menus open.
    pub popup: &'a mut Option<Popup>,
}

impl Arrange {
    pub fn new(session: &Session) -> Self {
        Self {
            categories: session.room_categories(),
            ..Self::default()
        }
    }

    pub fn dragging(&self) -> Option<&Drag> {
        self.drag.as_ref().filter(|d| d.active)
    }

    /// The section being dragged.
    pub fn dragged_section(&self) -> Option<&Section> {
        match &self.dragging()?.item {
            Dragged::Section(section) => Some(section),
            Dragged::Room(_) => None,
        }
    }

    /// The cursor entered (`true`) or left what drags `item`.
    fn hover(&mut self, item: Dragged, entered: bool) {
        if entered {
            self.hovered = Some(item);
        } else if self.hovered.as_ref() == Some(&item) {
            self.hovered = None;
        }
    }

    /// Stop a drag or naming a category; false if neither went on.
    pub fn escape(&mut self) -> bool {
        let open = self.drag.is_some() || self.naming.is_some();
        self.drag = None;
        self.target = None;
        self.naming = None;
        open
    }

    pub fn update(&mut self, message: Message, context: Context<'_>) -> Task<app::Message> {
        let space = context.space.cloned();
        // A choice in a menu closes it.
        let room_menu = |popup: &mut Popup| matches!(popup, Popup::RoomMenu(_));
        let category_menu = |popup: &mut Popup| matches!(popup, Popup::CategoryMenu(_));
        match message {
            Message::Categories(categories) => self.categories = categories,
            Message::Row(room, entered) => self.hover(Dragged::Room(room), entered),
            Message::Grip(section, entered) => self.hover(Dragged::Section(section), entered),
            Message::SectionHovered(section, true) => self.section_hovered = Some(section),
            Message::SectionHovered(section, false) => {
                if self.section_hovered.as_ref() == Some(&section) {
                    self.section_hovered = None;
                }
            }
            Message::NotesRow(hovered) => self.notes_hovered = hovered,
            Message::Pressed => {
                self.drag = self.hovered.clone().map(|item| Drag {
                    item,
                    start: None,
                    at: None,
                    active: false,
                });
            }
            Message::Moved(at) => {
                if let Some(drag) = &mut self.drag {
                    let start = *drag.start.get_or_insert(at);
                    drag.at = Some(at);
                    if !drag.active && start.distance(at) > DRAG_THRESHOLD {
                        drag.active = true;
                        // Sections are also dragged in the Show menu.
                        let section = matches!(drag.item, Dragged::Section(_));
                        context
                            .popup
                            .take_if(|popup| !(section && matches!(popup, Popup::ShowMenu)));
                    }
                }
            }
            Message::Released => {
                let drag = self.drag.take();
                if let (Some(drag), Some(target)) = (drag.filter(|d| d.active), self.target.take())
                {
                    return match drag.item {
                        Dragged::Room(room) => self.move_to(room, target, context),
                        Dragged::Section(section) => {
                            self.update(Message::MoveSection(section, target), context)
                        }
                    };
                }
                self.target = None;
            }
            Message::Target(section, true) => self.target = Some(section),
            Message::Target(section, false) => {
                if self.target.as_ref() == Some(&section) {
                    self.target = None;
                }
            }
            Message::ToggleCollapsed(section) => {
                return self.change(context.session, move |c| {
                    c.layout_mut(space.as_deref()).toggle_collapsed(&section)
                });
            }
            Message::Collapse(sections) => {
                return self.change(context.session, move |c| {
                    let layout = c.layout_mut(space.as_deref());
                    for (section, collapsed) in &sections {
                        layout.set_collapsed(section, *collapsed);
                    }
                });
            }
            Message::ShowMenu => *context.popup = Some(Popup::ShowMenu),
            Message::RoomMenu(room) => *context.popup = Some(Popup::RoomMenu(room)),
            Message::CategoryMenu(id) => *context.popup = Some(Popup::CategoryMenu(id)),
            Message::MoveTo(room, section) => {
                context.popup.take_if(room_menu);
                return self.move_to(room, section, context);
            }
            Message::Tag(room, tag) => {
                context.popup.take_if(room_menu);
                let Some(summary) = context.rooms.iter().find(|r| r.id == room) else {
                    return Task::none();
                };
                return set_tag(summary, tag, context.session);
            }
            Message::SaveCategory(edit) => {
                let Context {
                    session,
                    rooms,
                    grouping,
                    ..
                } = context;
                let layout = self.categories.layout(edit.space.as_deref());
                let placed: Vec<OwnedRoomId> = (edit.rooms.iter())
                    .filter(|room| {
                        edit.category.is_none()
                            || layout.category_of(room).map(|c| &c.id) != edit.category.as_ref()
                    })
                    .cloned()
                    .collect();
                // Rooms put into it are tagged as if dragged there.
                let mut tasks: Vec<_> = (rooms.iter())
                    .filter(|r| placed.contains(&r.id))
                    .map(|summary| set_tag(summary, tag_elsewhere(summary.tag, grouping), session))
                    .collect();
                tasks.push(self.change(session, move |c| edit.apply(c)));
                return Task::batch(tasks);
            }
            Message::Rename(id) => {
                context.popup.take_if(category_menu);
                let layout = self.categories.layout(space.as_deref());
                let name = layout.category(&id).map(|c| c.name.clone());
                self.naming = Some(Naming {
                    category: id,
                    name: name.unwrap_or_default(),
                });
                return iced::widget::operation::focus(NAME_INPUT);
            }
            Message::Name(name) => {
                if let Some(naming) = &mut self.naming {
                    naming.name = name;
                }
            }
            Message::SaveName => {
                let Some(naming) = self.naming.take() else {
                    return Task::none();
                };
                let name = naming.name.trim().to_owned();
                if name.is_empty() {
                    return Task::none();
                }
                return self.change(context.session, move |c| {
                    if let Some(category) = c
                        .layout_mut(space.as_deref())
                        .category_mut(&naming.category)
                    {
                        category.name = name;
                    }
                });
            }
            Message::Delete(id) => {
                context.popup.take_if(category_menu);
                return self.change(context.session, move |c| {
                    c.layout_mut(space.as_deref()).remove_category(&id)
                });
            }
            Message::MoveSection(section, to) => {
                return self.change(context.session, move |c| {
                    c.layout_mut(space.as_deref()).move_section(&section, &to)
                });
            }
            Message::Sort(id, sort) => {
                context.popup.take_if(category_menu);
                return self.change(context.session, move |c| {
                    if let Some(category) = c.layout_mut(space.as_deref()).category_mut(&id) {
                        category.sort = sort;
                    }
                });
            }
            Message::AtHome(at_home) => {
                return self.change(context.session, move |c| c.at_home = at_home);
            }
            Message::NotesAtTop(on) => {
                return self.change(context.session, move |c| c.notes_at_top = Some(on));
            }
            Message::Saved(Ok(())) => {}
            Message::Saved(Err(e)) => return failed(e),
        }
        Task::none()
    }

    /// Put a room into a section: into or out of a category of the shown
    /// space, and its tag set to match.
    fn move_to(
        &mut self,
        room: OwnedRoomId,
        section: Section,
        context: Context<'_>,
    ) -> Task<app::Message> {
        let Context {
            session,
            space,
            rooms,
            grouping,
            ..
        } = context;
        let Some(summary) = rooms.iter().find(|r| r.id == room) else {
            return Task::none();
        };
        let category = match &section {
            Section::Invites => return Task::none(),
            Section::Category(id) => Some(id.clone()),
            _ => None,
        };
        let tag = tag_in(&section, summary.tag, grouping);
        let mut tasks = vec![set_tag(summary, tag, session)];
        let layout = self.categories.layout(space.map(|s| s.as_ref()));
        if layout.category_of(&room).map(|c| &c.id) != category.as_ref() {
            let space = space.cloned();
            tasks.push(self.change(session, move |c| {
                c.layout_mut(space.as_deref())
                    .place(&room, category.as_deref())
            }));
        }
        Task::batch(tasks)
    }

    /// Change the categories here at once, and save them.
    fn change(
        &mut self,
        session: &Arc<Session>,
        change: impl FnOnce(&mut RoomCategories) + Clone + Send + 'static,
    ) -> Task<app::Message> {
        change.clone()(&mut self.categories);
        let session = session.clone();
        task::request(
            async move { session.change_room_categories(change).await },
            |result| app::Message::Arrange(Message::Saved(result)),
        )
    }

    /// The cursor while a room may be dragged.
    pub fn subscription(&self) -> Subscription<app::Message> {
        match self.drag {
            Some(_) => event::listen_with(|event, _status, _window| match event {
                iced::Event::Mouse(mouse::Event::CursorMoved { position }) => {
                    Some(app::Message::Arrange(Message::Moved(position)))
                }
                _ => None,
            }),
            None => Subscription::none(),
        }
    }
}

/// Tag a room; the room list shows it at once.
fn set_tag(summary: &RoomSummary, tag: RoomTag, session: &Arc<Session>) -> Task<app::Message> {
    if tag == summary.tag {
        return Task::none();
    }
    let (session, room) = (session.clone(), summary.id.clone());
    task::request(
        async move { session.room(&room)?.set_tag(tag).await },
        |result| app::Message::Arrange(Message::Saved(result)),
    )
}

/// The tag of a room with `tag` moved into `section`.
fn tag_in(section: &Section, tag: RoomTag, grouping: Grouping) -> RoomTag {
    match section {
        Section::Favourites => RoomTag::Favourite,
        Section::LowPriority => RoomTag::LowPriority,
        _ => tag_elsewhere(tag, grouping),
    }
}

/// The tag of a room with `tag` moved out of its tag's section: a tag that
/// decides the section goes.
fn tag_elsewhere(tag: RoomTag, grouping: Grouping) -> RoomTag {
    match tag {
        RoomTag::Favourite if grouping.favourites => RoomTag::None,
        RoomTag::LowPriority if grouping.low_priority => RoomTag::None,
        tag => tag,
    }
}

fn failed(error: String) -> Task<app::Message> {
    Task::done(app::Message::ArrangeFailed(error))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room(id: &str) -> OwnedRoomId {
        format!("!{id}:x").try_into().unwrap()
    }

    fn rooms_of(categories: &RoomCategories, id: &str) -> Vec<OwnedRoomId> {
        categories.home.category(id).unwrap().rooms.clone()
    }

    #[test]
    fn a_category_is_created_and_changed_with_its_rooms() {
        let mut categories = RoomCategories::default();
        let teaching = categories.home.add_category("Teaching");
        categories.home.place(&room("seminar"), Some(&teaching));

        // New, empty.
        CategoryEdit {
            space: None,
            category: None,
            name: "Empty".into(),
            rooms: Vec::new(),
        }
        .apply(&mut categories);
        assert_eq!(categories.home.categories[1].name, "Empty");
        assert!(categories.home.categories[1].rooms.is_empty());

        // New, taking a room from another category.
        CategoryEdit {
            space: None,
            category: None,
            name: "Papers".into(),
            rooms: vec![room("paper-a"), room("seminar")],
        }
        .apply(&mut categories);
        let papers = categories.home.categories[2].id.clone();
        assert_eq!(
            rooms_of(&categories, &papers),
            [room("paper-a"), room("seminar")]
        );
        assert!(rooms_of(&categories, &teaching).is_empty());

        // Renamed, one room out and one in; those staying keep their place.
        CategoryEdit {
            space: None,
            category: Some(papers.clone()),
            name: "Drafts".into(),
            rooms: vec![room("paper-b"), room("seminar")],
        }
        .apply(&mut categories);
        let drafts = categories.home.category(&papers).unwrap();
        assert_eq!(drafts.name, "Drafts");
        assert_eq!(drafts.rooms, [room("seminar"), room("paper-b")]);
        assert_eq!(categories.home.category_of(&room("paper-a")), None);
    }
}
