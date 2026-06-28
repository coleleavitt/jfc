use crate::app::App;
use jfc_engine::runtime::{FrontendOpenPanelRequest, RuntimeActionSource};
use jfc_plugin_sdk::{RuntimeActionDescriptor, UiMutationScope, UiWidgetDescriptor};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InfoSidebarWidgetFocusStep {
    Previous,
    Next,
}

pub(super) fn focus_info_sidebar_widget_request(
    app: &mut App,
    request: &FrontendOpenPanelRequest,
) -> Option<RuntimeActionDescriptor> {
    let Some(widget) = request.widget.as_ref() else {
        return None;
    };

    let Some(runtime_action_id) = runtime_action_id_for_info_sidebar_widget(
        app,
        widget.plugin_id.as_str(),
        widget.widget_id.as_str(),
        &request.source,
    ) else {
        return None;
    };

    app.info_sidebar
        .focus_widget(widget.plugin_id.clone(), widget.widget_id.clone());
    runtime_action_id.and_then(|runtime_action_id| {
        runtime_action_for_widget(
            app,
            widget.plugin_id.as_str(),
            runtime_action_id.as_str(),
            &request.source,
        )
    })
}

pub(super) fn move_info_sidebar_widget_focus(
    app: &mut App,
    step: InfoSidebarWidgetFocusStep,
) -> bool {
    let widgets = sorted_info_sidebar_widgets(&app.plugins.ui_widget_descriptors);
    if widgets.is_empty() {
        app.info_sidebar.clear_widget_focus();
        return false;
    }

    let current_index = app
        .info_sidebar
        .focused_widget
        .as_ref()
        .and_then(|focused| {
            widgets.iter().position(|widget| {
                focused.plugin_id.as_str() == widget.plugin_id.as_str()
                    && focused.widget_id.as_str() == widget.id.as_str()
            })
        });
    let next_index = match (step, current_index) {
        (InfoSidebarWidgetFocusStep::Next, Some(index)) => (index + 1) % widgets.len(),
        (InfoSidebarWidgetFocusStep::Next, None) => 0,
        (InfoSidebarWidgetFocusStep::Previous, Some(0) | None) => widgets.len() - 1,
        (InfoSidebarWidgetFocusStep::Previous, Some(index)) => index - 1,
    };
    let plugin_id = widgets[next_index].plugin_id.as_str().to_owned();
    let widget_id = widgets[next_index].id.clone();
    app.info_sidebar.focus_widget(plugin_id, widget_id);
    true
}

pub(super) fn focused_info_sidebar_widget_descriptor(app: &mut App) -> Option<UiWidgetDescriptor> {
    let focused = app.info_sidebar.focused_widget.as_ref()?;
    let Some(widget) = app.plugins.ui_widget_descriptors.iter().find(|widget| {
        widget.scope == UiMutationScope::InfoSidebar
            && widget.plugin_id.as_str() == focused.plugin_id.as_str()
            && widget.id.as_str() == focused.widget_id.as_str()
    }) else {
        app.info_sidebar.clear_widget_focus();
        return None;
    };
    Some(widget.clone())
}

pub(super) fn runtime_action_for_widget_descriptor(
    app: &App,
    widget: &UiWidgetDescriptor,
) -> Option<RuntimeActionDescriptor> {
    let runtime_action_id = widget.runtime_action_id.as_deref()?;
    runtime_action_for_widget_id(app, widget.plugin_id.as_str(), runtime_action_id)
}

fn runtime_action_id_for_info_sidebar_widget(
    app: &mut App,
    plugin_id: &str,
    widget_id: &str,
    source: &RuntimeActionSource,
) -> Option<Option<String>> {
    let Some(widget) = app
        .plugins
        .ui_widget_descriptors
        .iter()
        .find(|widget| is_info_sidebar_widget(widget, plugin_id, widget_id))
    else {
        app.info_sidebar.clear_widget_focus();
        tracing::warn!(
            target: "jfc::palette",
            plugin = source.plugin_id.as_str(),
            action = source.action_id.as_str(),
            widget_plugin = plugin_id,
            widget_id,
            "runtime action targeted an unknown info-sidebar widget"
        );
        return None;
    };
    Some(widget.runtime_action_id.clone())
}

fn runtime_action_for_widget(
    app: &App,
    plugin_id: &str,
    runtime_action_id: &str,
    source: &RuntimeActionSource,
) -> Option<RuntimeActionDescriptor> {
    let runtime_action = runtime_action_for_widget_id(app, plugin_id, runtime_action_id);
    if runtime_action.is_none() {
        tracing::warn!(
            target: "jfc::palette",
            plugin = source.plugin_id.as_str(),
            action = source.action_id.as_str(),
            widget_plugin = plugin_id,
            widget_action = runtime_action_id,
            "focused plugin widget references a missing runtime action"
        );
    }
    runtime_action
}

fn runtime_action_for_widget_id(
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

fn sorted_info_sidebar_widgets(widgets: &[UiWidgetDescriptor]) -> Vec<&UiWidgetDescriptor> {
    let mut panel_widgets = widgets
        .iter()
        .filter(|widget| widget.scope == UiMutationScope::InfoSidebar)
        .collect::<Vec<_>>();
    panel_widgets.sort_by(|left, right| {
        right
            .priority
            .cmp(&left.priority)
            .then_with(|| left.plugin_id.as_str().cmp(right.plugin_id.as_str()))
            .then_with(|| left.id.cmp(&right.id))
    });
    panel_widgets
}

fn is_info_sidebar_widget(widget: &UiWidgetDescriptor, plugin_id: &str, widget_id: &str) -> bool {
    widget.scope == UiMutationScope::InfoSidebar
        && widget.plugin_id.as_str() == plugin_id
        && widget.id.as_str() == widget_id
}
