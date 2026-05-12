// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]
#![cfg(feature = "ai")]

use std::any::type_name;

use zk_app::{
    AiFacade, ChatChunk, ChatRequest, ConversationDetail, ConversationDto, ListConversationsParams,
    RecomputeLocalSummary, SemanticSearchRequest,
};

#[test]
fn ai_facade_types_are_available_from_crate_root() {
    assert!(!type_name::<AiFacade>().is_empty());
    assert!(!type_name::<SemanticSearchRequest>().is_empty());
    assert!(!type_name::<RecomputeLocalSummary>().is_empty());
    assert!(!type_name::<ChatRequest>().is_empty());
    assert!(!type_name::<ChatChunk>().is_empty());
    assert!(!type_name::<ListConversationsParams>().is_empty());
    assert!(!type_name::<ConversationDto>().is_empty());
    assert!(!type_name::<ConversationDetail>().is_empty());
}
