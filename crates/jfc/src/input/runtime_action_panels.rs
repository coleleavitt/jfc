use crate::app::App;
use jfc_engine::runtime::{FrontendOpenPanelRequest, RuntimeActionSource};
use jfc_plugin_sdk::{RuntimeActionDescriptor, UiMutationScope, UiPanelDescriptor};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InfoSidebarPanelFocusStep {
    Previous,
    Next,
}

struct PanelFocusTarget<'a> {
    plugin_id: &'a str,
    panel_id: &'a str,
    source: &'a RuntimeActionSource,
}

pub(super) fn focus_info_sidebar_panel_request(
    app: &mut App,
    request: &FrontendOpenPanelRequest,
) -> Option<RuntimeActionDescriptor> {
    let Some(panel) = request.panel.as_ref() else {
        return None;
    };
    let target = PanelFocusTarget {
        plugin_id: panel.plugin_id.as_str(),
        panel_id: panel.panel_id.as_str(),
        source: &request.source,
    };

    let Some(runtime_action_id) = runtime_action_id_for_info_sidebar_panel(app, target) else {
        return None;
    };

    app.info_sidebar
        .focus_panel(panel.plugin_id.clone(), panel.panel_id.clone());
    let runtime_action_id = runtime_action_id?;
    let runtime_action =
        runtime_action_for_panel_id(app, panel.plugin_id.as_str(), runtime_action_id.as_str());
    if runtime_action.is_none() {
        tracing::warn!(
            target: "jfc::palette",
            plugin = request.source.plugin_id.as_str(),
            action = request.source.action_id.as_str(),
            panel_plugin = panel.plugin_id.as_str(),
            panel_action = runtime_action_id.as_str(),
            "focused plugin panel references a missing runtime action"
        );
    }
    runtime_action
}

pub(super) fn move_info_sidebar_panel_focus(
    app: &mut App,
    step: InfoSidebarPanelFocusStep,
) -> bool {
    let panels = sorted_info_sidebar_panels(&app.plugins.ui_panel_descriptors);
    if panels.is_empty() {
        app.info_sidebar.clear_panel_focus();
        return false;
    }

    let current_index = app.info_sidebar.focused_panel.as_ref().and_then(|focused| {
        panels.iter().position(|panel| {
            focused.plugin_id.as_str() == panel.plugin_id.as_str()
                && focused.panel_id.as_str() == panel.id.as_str()
        })
    });
    let next_index = match (step, current_index) {
        (InfoSidebarPanelFocusStep::Next, Some(index)) => (index + 1) % panels.len(),
        (InfoSidebarPanelFocusStep::Next, None) => 0,
        (InfoSidebarPanelFocusStep::Previous, Some(0) | None) => panels.len() - 1,
        (InfoSidebarPanelFocusStep::Previous, Some(index)) => index - 1,
    };
    let plugin_id = panels[next_index].plugin_id.as_str().to_owned();
    let panel_id = panels[next_index].id.clone();
    app.info_sidebar.focus_panel(plugin_id, panel_id);
    true
}

pub(super) fn focused_info_sidebar_panel_descriptor(app: &mut App) -> Option<UiPanelDescriptor> {
    let focused = app.info_sidebar.focused_panel.as_ref()?;
    let Some(panel) = app.plugins.ui_panel_descriptors.iter().find(|panel| {
        panel.scope == UiMutationScope::InfoSidebar
            && panel.plugin_id.as_str() == focused.plugin_id.as_str()
            && panel.id.as_str() == focused.panel_id.as_str()
    }) else {
        app.info_sidebar.clear_panel_focus();
        return None;
    };
    Some(panel.clone())
}

pub(super) fn runtime_action_for_panel_descriptor(
    app: &App,
    panel: &UiPanelDescriptor,
) -> Option<RuntimeActionDescriptor> {
    let runtime_action_id = panel.runtime_action_id.as_deref()?;
    runtime_action_for_panel_id(app, panel.plugin_id.as_str(), runtime_action_id)
}

fn runtime_action_id_for_info_sidebar_panel(
    app: &mut App,
    target: PanelFocusTarget<'_>,
) -> Option<Option<String>> {
    let Some(panel) = app
        .plugins
        .ui_panel_descriptors
        .iter()
        .find(|panel| is_info_sidebar_panel(panel, target.plugin_id, target.panel_id))
    else {
        app.info_sidebar.clear_panel_focus();
        tracing::warn!(
            target: "jfc::palette",
            plugin = target.source.plugin_id.as_str(),
            action = target.source.action_id.as_str(),
            panel_plugin = target.plugin_id,
            panel_id = target.panel_id,
            "runtime action targeted an unknown info-sidebar panel"
        );
        return None;
    };
    Some(panel.runtime_action_id.clone())
}

fn runtime_action_for_panel_id(
    app: &App,
    plugin_id: &str,
    runtime_action_id: &str,
) -> Option<RuntimeActionDescriptor> {
    app.plugins
        .runtime_action_descriptors
        .iter()
        .find(|candidate| {
            candidate.plugin_id.as_str() == plugin_id && candidate.id == runtime_action_id
        })
        .cloned()
}

fn sorted_info_sidebar_panels(panels: &[UiPanelDescriptor]) -> Vec<&UiPanelDescriptor> {
    let mut info_panels = panels
        .iter()
        .filter(|panel| panel.scope == UiMutationScope::InfoSidebar)
        .collect::<Vec<_>>();
    info_panels.sort_by(|left, right| {
        right
            .priority
            .cmp(&left.priority)
            .then_with(|| left.plugin_id.as_str().cmp(right.plugin_id.as_str()))
            .then_with(|| left.id.cmp(&right.id))
    });
    info_panels
}

fn is_info_sidebar_panel(panel: &UiPanelDescriptor, plugin_id: &str, panel_id: &str) -> bool {
    panel.scope == UiMutationScope::InfoSidebar
        && panel.plugin_id.as_str() == plugin_id
        && panel.id.as_str() == panel_id
}
