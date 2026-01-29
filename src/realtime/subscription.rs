use async_graphql::{Subscription, SimpleObject};
use futures_util::Stream;
use crate::realtime::bus::EventBus;
use std::sync::Arc;

pub struct SubscriptionRoot;

#[Subscription]
impl SubscriptionRoot {
    async fn events(&self, ctx: &async_graphql::Context<'_>) -> impl Stream<Item = GraphQLEvent> {
        let bus = ctx.data::<Arc<EventBus>>().expect("EventBus not found in context");
        let mut rx = bus.subscribe();
        
        async_stream::stream! {
            while let Ok(msg) = rx.recv().await {
                yield GraphQLEvent {
                    uid: msg.uid.to_string(),
                    field: msg.field,
                    value: msg.new_value_str,
                };
            }
        }
    }
}

#[derive(SimpleObject)]
pub struct GraphQLEvent {
    uid: String,
    field: String,
    value: Option<String>,
}
