//! A (value, label) option for `Select`, plus helpers to build and read selects.

use gpui_kit::component::select::{SelectEvent, SelectState};
use gpui_kit::component::IndexPath;
use gpui_kit::*;

#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    pub value: String,
    pub label: SharedString,
}

impl Choice {
    pub fn new(value: impl Into<String>, label: impl Into<SharedString>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
        }
    }
}

impl gpui_kit::component::searchable_list::SearchableListItem for Choice {
    type Value = String;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &String {
        &self.value
    }
}

pub type ChoiceSelect = SelectState<Vec<Choice>>;

/// Build a select showing `options` with `selected` (a value) preselected.
pub fn new_select(
    options: Vec<Choice>,
    selected: &str,
    window: &mut Window,
    cx: &mut Context<'_, ChoiceSelect>,
) -> ChoiceSelect {
    let ix = options
        .iter()
        .position(|c| c.value == selected)
        .map(|i| IndexPath::default().row(i));
    SelectState::new(options, ix, window, cx)
}

pub fn confirmed_value(event: &SelectEvent<Vec<Choice>>) -> Option<String> {
    let SelectEvent::Confirm(v) = event;
    v.clone()
}
