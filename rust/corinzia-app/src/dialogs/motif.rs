//! The Motif card: naming a motif taken from the score and saying what it
//! is (its roles, free tags, a note in one's own words), with what can be
//! read off its notes shown for reference.

use crate::app::*;
use crate::{motif_view, panels, theme};
use corinzia_model::motif::{Motif, Role};
use iced::widget::{button, column, row, text, text_input, Canvas};
use iced::{Element, Length};

pub(crate) struct MotifDialog {
    pub(crate) draft: Motif,
    /// Not yet in the bank.
    pub(crate) new: bool,
    pub(crate) name: String,
    pub(crate) tags: String,
    pub(crate) comment: String,
    pub(crate) facts: Vec<(&'static str, String)>,
    pub(crate) error: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) enum MotifMsg {
    Name(String),
    Role(Role),
    Tags(String),
    Comment(String),
    Save,
    Delete,
    Cancel,
}

impl MotifDialog {
    pub(crate) fn new(project: &corinzia_model::Project, m: Motif, new: bool) -> MotifDialog {
        MotifDialog { name: m.name.clone(), tags: m.tags.join(", "), comment: m.comment.clone(), facts: motif_view::facts(project, &m), draft: m, new, error: None }
    }
}

pub(crate) fn motif_card(d: &MotifDialog) -> Element<'_, Message> {
    let bold = iced::Font { weight: iced::font::Weight::Bold, ..iced::Font::DEFAULT };
    let mm = Message::Motif;
    let label = |t: &'static str| text(t).size(12).style(theme::dim);
    let mut facts = column![].spacing(2);
    for (k, v) in &d.facts {
        facts = facts.push(row![text(*k).size(11).style(theme::faint).width(64), text(v.as_str()).size(11)].spacing(6));
    }
    let mut roles = column![].spacing(4);
    for chunk in Role::ALL.chunks(4) {
        let mut r = row![].spacing(4);
        for &role in chunk {
            let on = d.draft.roles.contains(&role);
            r = r.push(button(text(role.name()).size(12)).padding([4, 8]).width(Length::Fill).style(theme::tool(on)).on_press(mm(MotifMsg::Role(role))));
        }
        roles = roles.push(r);
    }
    let mut card = column![
        text(if d.new { "New motif" } else { "Motif" }).size(18).font(bold),
        Canvas::new(motif_view::Roll { motif: &d.draft }).width(Length::Fill).height(80),
        facts,
        column![label("Name"), text_input("e.g. Main theme, opening call", &d.name).on_input(move |v| mm(MotifMsg::Name(v))).padding(5).size(14)].spacing(3),
        column![label("What it is (any that apply)"), roles].spacing(3),
        column![label("Tags (comma separated: character, mood, style…)"), text_input("lyrical, yearning, rising", &d.tags).on_input(move |v| mm(MotifMsg::Tags(v))).padding(5).size(14)].spacing(3),
        column![label("Notes"), text_input("In your own words: where it goes, what answers it…", &d.comment).on_input(move |v| mm(MotifMsg::Comment(v))).on_submit(mm(MotifMsg::Save)).padding(5).size(14)].spacing(3),
    ]
    .spacing(10)
    .width(560);
    if let Some(e) = &d.error {
        card = card.push(text(e).size(12).style(theme::danger));
    }
    let mut buttons = row![].spacing(8);
    if !d.new {
        buttons = buttons.push(button(text("Delete").size(13)).padding([6, 14]).style(theme::tool(false)).on_press(mm(MotifMsg::Delete)));
    }
    buttons = buttons
        .push(panels::spacer())
        .push(button(text("Cancel").size(13)).padding([6, 14]).style(theme::tool(false)).on_press(mm(MotifMsg::Cancel)))
        .push(button(text(if d.new { "Add to the bank" } else { "Save" }).size(13)).padding([6, 14]).style(theme::primary).on_press(mm(MotifMsg::Save)));
    card.push(buttons).into()
}

impl App {
    pub(crate) fn motif_msg(&mut self, m: MotifMsg) {
        let Some(d) = self.dialogs.motif.as_mut() else { return };
        d.error = None;
        match m {
            MotifMsg::Name(v) => d.name = v,
            MotifMsg::Role(r) => {
                if let Some(i) = d.draft.roles.iter().position(|x| *x == r) {
                    d.draft.roles.remove(i);
                } else {
                    d.draft.roles.push(r);
                    d.draft.roles.sort();
                }
            }
            MotifMsg::Tags(v) => d.tags = v,
            MotifMsg::Comment(v) => d.comment = v,
            MotifMsg::Cancel => self.dialogs.motif = None,
            MotifMsg::Delete => {
                let id = d.draft.id;
                self.dialogs.motif = None;
                self.editor.delete_motif(id);
            }
            MotifMsg::Save => {
                if d.name.trim().is_empty() {
                    d.error = Some("Name the motif".into());
                    return;
                }
                let mut m = d.draft.clone();
                m.name = d.name.trim().to_string();
                m.tags = d.tags.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();
                m.comment = d.comment.trim().to_string();
                self.dialogs.motif = None;
                self.editor.save_motif(m);
            }
        }
    }
}
