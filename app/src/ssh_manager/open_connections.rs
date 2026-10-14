use std::collections::HashMap;

use uuid::Uuid;
use warpui::{
    AppContext, Entity, EntityId, ModelContext, ModelHandle, SingletonEntity, ViewHandle,
    WeakViewHandle,
};

use crate::terminal::TerminalView;

/// SSH manager connections that are currently up, by the terminal view running them.
#[derive(Default)]
pub struct SshOpenConnections {
    /// The profile each terminal is connected to, or `None` for a quick connect.
    connections: HashMap<EntityId, (WeakViewHandle<TerminalView>, Option<Uuid>)>,
}

impl SshOpenConnections {
    pub fn connected(
        terminal: WeakViewHandle<TerminalView>,
        profile_id: Option<Uuid>,
        ctx: &mut AppContext,
    ) {
        Self::update(ctx, |me, ctx| {
            me.connections.insert(terminal.id(), (terminal, profile_id));
            ctx.notify();
        });
    }

    pub fn disconnected(view_id: EntityId, ctx: &mut AppContext) {
        Self::update(ctx, |me, ctx| {
            if me.connections.remove(&view_id).is_some() {
                ctx.notify();
            }
        });
    }

    pub fn handle_or_init(ctx: &mut AppContext) -> ModelHandle<Self> {
        if !ctx.has_singleton_model::<Self>() {
            ctx.add_singleton_model(|_| Self::default());
        }
        Self::handle(ctx)
    }

    fn update(ctx: &mut AppContext, f: impl FnOnce(&mut Self, &mut ModelContext<Self>)) {
        Self::handle_or_init(ctx).update(ctx, f);
    }

    pub fn open_count(profile_id: Uuid, app: &AppContext) -> usize {
        Self::terminals(profile_id, app).len()
    }

    pub fn terminals(profile_id: Uuid, app: &AppContext) -> Vec<ViewHandle<TerminalView>> {
        if !app.has_singleton_model::<Self>() {
            return Vec::new();
        }
        Self::as_ref(app)
            .connections
            .values()
            .filter(|(_, id)| *id == Some(profile_id))
            .filter_map(|(terminal, _)| terminal.upgrade(app))
            .collect()
    }
}

impl Entity for SshOpenConnections {
    type Event = ();
}

impl SingletonEntity for SshOpenConnections {}
