// Copyright (c) 2023 - 2026 Restate Software, Inc., Restate GmbH.
// All rights reserved.
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

use super::*;
use std::ops::RangeInclusive;

use restate_invoker_api::Effect;
use restate_invoker_api::invocation_reader::InvocationReader;
use restate_types::identifiers::PartitionKey;

/// Tree of [InvocationStateMachine] held by the [Service].
#[derive(Debug)]
pub(super) struct InvocationStateMachineManager<SR> {
    runtime: Option<RuntimeInvocationStateMachineCoordinator<SR>>,
}

impl<SR> Default for InvocationStateMachineManager<SR> {
    fn default() -> Self {
        InvocationStateMachineManager { runtime: None }
    }
}

#[derive(Debug)]
struct RuntimeInvocationStateMachineCoordinator<IR> {
    output_tx: mpsc::Sender<Box<Effect>>,
    invocation_state_machines: HashMap<InvocationId, InvocationStateMachine>,
    partition_key_range: RangeInclusive<PartitionKey>,
    storage_reader: IR,
}

impl<IR> InvocationStateMachineManager<IR>
where
    IR: InvocationReader + Clone + Send + Sync + 'static,
{
    #[inline]
    pub(super) fn has_runtime(&self) -> bool {
        self.runtime.is_some()
    }

    #[inline]
    pub(super) fn runtime_storage_reader(&self) -> Option<&IR> {
        self.runtime.as_ref().map(|p| &p.storage_reader)
    }

    #[inline]
    pub(super) fn resolve_runtime_sender(&self) -> Option<&mpsc::Sender<Box<Effect>>> {
        self.runtime.as_ref().map(|p| &p.output_tx)
    }

    #[inline]
    pub(super) fn resolve_invocation(
        &mut self,
        invocation_id: &InvocationId,
    ) -> Option<(&mpsc::Sender<Box<Effect>>, &mut InvocationStateMachine)> {
        self.resolve_runtime().and_then(|p| {
            p.invocation_state_machines
                .get_mut(invocation_id)
                .map(|ism| (&p.output_tx, ism))
        })
    }

    #[inline]
    pub(super) fn handle_for_invocation<R>(
        &mut self,
        invocation_id: &InvocationId,
        f: impl FnOnce(&mpsc::Sender<Box<Effect>>, &mut InvocationStateMachine) -> R,
    ) -> Option<R> {
        if let Some((tx, ism)) = self.resolve_invocation(invocation_id) {
            Some(f(tx, ism))
        } else {
            // If no state machine
            trace!("No state machine found for selected server header");
            None
        }
    }

    #[inline]
    pub(super) fn remove_invocation(
        &mut self,
        invocation_id: &InvocationId,
    ) -> Option<(&mpsc::Sender<Box<Effect>>, &IR, InvocationStateMachine)> {
        self.resolve_runtime().and_then(|p| {
            p.invocation_state_machines
                .remove(invocation_id)
                .map(|ism| (&p.output_tx, &p.storage_reader, ism))
        })
    }

    #[inline]
    pub(super) fn remove_runtime(
        &mut self,
    ) -> Option<HashMap<InvocationId, InvocationStateMachine>> {
        self.runtime.take().map(|p| p.invocation_state_machines)
    }

    #[inline]
    pub(super) fn register_runtime(
        &mut self,
        partition_key_range: RangeInclusive<PartitionKey>,
        storage_reader: IR,
        sender: mpsc::Sender<Box<Effect>>,
    ) {
        self.runtime = Some(RuntimeInvocationStateMachineCoordinator {
            output_tx: sender,
            invocation_state_machines: Default::default(),
            partition_key_range,
            storage_reader,
        });
    }

    #[inline]
    pub(super) fn register_invocation(&mut self, id: InvocationId, ism: InvocationStateMachine) {
        self.resolve_runtime()
            .expect("Cannot register an invocation on an unknown runtime")
            .invocation_state_machines
            .insert(id, ism);
    }

    #[inline]
    pub(super) fn runtime_matches_keys(&self, keys: &RangeInclusive<PartitionKey>) -> bool {
        self.runtime
            .as_ref()
            .map(|coordinator| {
                coordinator.partition_key_range.start() <= keys.end()
                    && keys.start() <= coordinator.partition_key_range.end()
            })
            .unwrap_or(false)
    }

    #[inline]
    fn resolve_runtime(&mut self) -> Option<&mut RuntimeInvocationStateMachineCoordinator<IR>> {
        self.runtime.as_mut()
    }
}
