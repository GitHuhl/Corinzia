//! The Layout card: the score's and the parts' page layouts, and the page texts.

use crate::*;
use iced::widget::column;

/// The Layout card: the score's and the parts' page layouts as typed,
/// and the page texts.
pub(crate) struct LayoutDialog {
    pub(crate) parts: bool,
    pub(crate) fields: [LayoutFields; 2], // score, parts
    pub(crate) texts: [String; 4],        // subtitle, lyricist, arranger, copyright
    pub(crate) error: Option<String>,
}

#[derive(Clone)]
pub(crate) struct LayoutFields {
    pub(crate) size: String, // a name from PAGE_SIZES, or "Custom"
    pub(crate) width: String,
    pub(crate) height: String,
    pub(crate) landscape: bool,
    pub(crate) staff: String,
    pub(crate) margin_y: String,
    pub(crate) margin_x: String,
    pub(crate) spacing: String, // percent
    pub(crate) staff_distance: String,
    pub(crate) system_gap: String,
    pub(crate) bars: String,
    pub(crate) page_numbers: bool,
    pub(crate) condense: bool,
}

impl LayoutFields {
    pub(crate) fn of(l: &corinzia_model::Layout) -> LayoutFields {
        let n = |v: f64| {
            let s = format!("{v:.2}");
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        };
        let size = corinzia_model::PAGE_SIZES.iter().find(|(_, w, h)| (w - l.page_width_mm).abs() < 0.05 && (h - l.page_height_mm).abs() < 0.05).map(|p| p.0).unwrap_or("Custom");
        LayoutFields {
            size: size.to_string(),
            width: n(l.page_width_mm),
            height: n(l.page_height_mm),
            landscape: l.landscape,
            staff: n(l.staff_mm),
            margin_y: n(l.margin_y_mm),
            margin_x: n(l.margin_x_mm),
            spacing: n(l.spacing * 100.0),
            staff_distance: n(l.staff_distance),
            system_gap: n(l.system_gap),
            bars: if l.bars_per_system == 0 { String::new() } else { l.bars_per_system.to_string() },
            page_numbers: l.page_numbers,
            condense: l.condense,
        }
    }

    /// The layout typed, or what is wrong with it.
    pub(crate) fn layout(&self) -> Result<corinzia_model::Layout, String> {
        let num = |s: &str, what: &str, lo: f64, hi: f64| -> Result<f64, String> {
            s.trim().replace(',', ".").parse::<f64>().ok().filter(|v| (lo..=hi).contains(v)).ok_or(format!("{what}: a number from {lo} to {hi}"))
        };
        Ok(corinzia_model::Layout {
            page_width_mm: num(&self.width, "Page width (mm)", 50.0, 1000.0)?,
            page_height_mm: num(&self.height, "Page height (mm)", 50.0, 1000.0)?,
            landscape: self.landscape,
            staff_mm: num(&self.staff, "Staff size (mm)", 3.0, 12.0)?,
            margin_y_mm: num(&self.margin_y, "Top and bottom margins (mm)", 0.0, 60.0)?,
            margin_x_mm: num(&self.margin_x, "Side margins (mm)", 0.0, 60.0)?,
            spacing: num(&self.spacing, "Note spacing (%)", 50.0, 250.0)? / 100.0,
            staff_distance: num(&self.staff_distance, "Staff distance (spaces)", 5.0, 30.0)?,
            system_gap: num(&self.system_gap, "System gap (spaces)", 0.0, 40.0)?,
            bars_per_system: if self.bars.trim().is_empty() { 0 } else { num(&self.bars, "Bars per system", 1.0, 64.0)? as u32 },
            page_numbers: self.page_numbers,
            condense: self.condense,
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) enum LayoutMsg {
    Tab(bool),
    Size(String),
    Field(LayoutField, String),
    Landscape(bool),
    PageNumbers(bool),
    Condense(bool),
    Text(usize, String),
    Defaults,
    Apply,
    Cancel,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum LayoutField {
    Width,
    Height,
    Staff,
    MarginY,
    MarginX,
    Spacing,
    StaffDistance,
    SystemGap,
    Bars,
}

/// The Layout card: the page of the score or of the parts, and the texts.
pub(crate) fn layout_card(d: &LayoutDialog) -> Element<'_, Message> {
    let bold = iced::Font { weight: iced::font::Weight::Bold, ..iced::Font::DEFAULT };
    let f = &d.fields[usize::from(d.parts)];
    let lm = |m: LayoutMsg| Message::Layout(m);
    let field = |label: &'static str, value: &str, which: LayoutField| {
        column![text(label).size(12).style(theme::dim), text_input("", value).on_input(move |v| lm(LayoutMsg::Field(which, v))).on_submit(lm(LayoutMsg::Apply)).padding(5).size(14)].spacing(3).width(Length::Fill)
    };
    let toggle = |label: &'static str, on: bool, m: LayoutMsg| button(text(label).size(13)).padding([5, 10]).style(theme::tool(on)).on_press(lm(m));
    let mut sizes: Vec<String> = corinzia_model::PAGE_SIZES.iter().map(|p| p.0.to_string()).collect();
    sizes.push("Custom".into());
    let size = iced::widget::pick_list(sizes, Some(f.size.clone()), |s| Message::Layout(LayoutMsg::Size(s))).text_size(13).padding([5, 9]).width(Length::Fill);
    let texts = ["Subtitle", "Lyricist", "Arranger", "Copyright"];
    let text_fields = column(texts.iter().enumerate().map(|(i, label)| {
        row![text(*label).size(12).style(theme::dim).width(80), text_input("", &d.texts[i]).on_input(move |v| lm(LayoutMsg::Text(i, v))).on_submit(lm(LayoutMsg::Apply)).padding(5).size(14)].spacing(8).align_y(iced::Alignment::Center).into()
    }))
    .spacing(6);
    let mut card = column![
        row![text("Layout").size(18).font(bold), panels::spacer(), segmented(vec![("Score", !d.parts, lm(LayoutMsg::Tab(false))), ("Parts", d.parts, lm(LayoutMsg::Tab(true)))])].align_y(iced::Alignment::Center),
        row![column![text("Page").size(12).style(theme::dim), size].spacing(3).width(Length::Fill), field("Width (mm)", &f.width, LayoutField::Width), field("Height (mm)", &f.height, LayoutField::Height)].spacing(10),
        row![toggle("Portrait", !f.landscape, LayoutMsg::Landscape(false)), toggle("Landscape", f.landscape, LayoutMsg::Landscape(true)), panels::spacer(), toggle("Page numbers", f.page_numbers, LayoutMsg::PageNumbers(!f.page_numbers)), { let e: iced::Element<'_, Message> = if d.parts { panels::spacer().into() } else { toggle("Condense", f.condense, LayoutMsg::Condense(!f.condense)).into() }; e }].spacing(6),
        row![field("Staff size (mm)", &f.staff, LayoutField::Staff), field("Margins top/bottom (mm)", &f.margin_y, LayoutField::MarginY), field("Margins sides (mm)", &f.margin_x, LayoutField::MarginX)].spacing(10),
        row![field("Note spacing (%)", &f.spacing, LayoutField::Spacing), field("Staff distance (spaces)", &f.staff_distance, LayoutField::StaffDistance), field("System gap (spaces)", &f.system_gap, LayoutField::SystemGap), field("Bars per system", &f.bars, LayoutField::Bars)].spacing(10),
        text("Breaks: Enter puts a system break before the selected bar, Ctrl+Enter a page break (in the part shown, or the full score).").size(11).style(theme::dim),
        text("Page texts").size(12).style(theme::dim),
        text_fields,
    ]
    .spacing(10)
    .width(600);
    if let Some(e) = &d.error {
        card = card.push(text(e).size(12).style(theme::danger));
    }
    card.push(
        row![
            button(text(if d.parts { "Parts defaults" } else { "Score defaults" }).size(13)).padding([6, 14]).style(theme::tool(false)).on_press(lm(LayoutMsg::Defaults)),
            panels::spacer(),
            button(text("Cancel").size(13)).padding([6, 14]).style(theme::tool(false)).on_press(lm(LayoutMsg::Cancel)),
            button(text("Apply").size(13)).padding([6, 14]).style(theme::primary).on_press(lm(LayoutMsg::Apply)),
        ]
        .spacing(8),
    )
    .into()
}

impl App {
    pub(crate) fn layout_msg(&mut self, m: LayoutMsg) {
        let Some(d) = self.layout.as_mut() else { return };
        d.error = None;
        let k = usize::from(d.parts);
        let f = &mut d.fields[k];
        match m {
            LayoutMsg::Tab(parts) => d.parts = parts,
            LayoutMsg::Size(name) => {
                if let Some((_, w, h)) = corinzia_model::PAGE_SIZES.iter().find(|p| p.0 == name) {
                    (f.width, f.height) = (w.to_string(), h.to_string());
                }
                f.size = name;
            }
            LayoutMsg::Field(which, v) => {
                let slot = match which {
                    LayoutField::Width | LayoutField::Height => {
                        f.size = "Custom".into();
                        if matches!(which, LayoutField::Width) { &mut f.width } else { &mut f.height }
                    }
                    LayoutField::Staff => &mut f.staff,
                    LayoutField::MarginY => &mut f.margin_y,
                    LayoutField::MarginX => &mut f.margin_x,
                    LayoutField::Spacing => &mut f.spacing,
                    LayoutField::StaffDistance => &mut f.staff_distance,
                    LayoutField::SystemGap => &mut f.system_gap,
                    LayoutField::Bars => &mut f.bars,
                };
                *slot = v;
            }
            LayoutMsg::Landscape(on) => f.landscape = on,
            LayoutMsg::PageNumbers(on) => f.page_numbers = on,
            LayoutMsg::Condense(on) => f.condense = on,
            LayoutMsg::Text(i, v) => d.texts[i] = v,
            LayoutMsg::Defaults => *f = LayoutFields::of(&if d.parts { corinzia_model::Layout::parts() } else { corinzia_model::Layout::default() }),
            LayoutMsg::Cancel => self.layout = None,
            LayoutMsg::Apply => {
                let (score, parts) = match (d.fields[0].layout(), d.fields[1].layout()) {
                    (Ok(a), Ok(b)) => (a, b),
                    (Err(e), _) => {
                        d.error = Some(format!("Score: {e}"));
                        return;
                    }
                    (_, Err(e)) => {
                        d.error = Some(format!("Parts: {e}"));
                        return;
                    }
                };
                let texts = d.texts.clone();
                self.layout = None;
                self.editor.set_layout(score, parts, texts);
            }
        }
    }
}
