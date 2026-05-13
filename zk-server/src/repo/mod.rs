//! `SQLite` repository helpers over the server schema.

mod ai_jobs;
mod assets;
mod chunks;
mod conversations;
mod cursors;
mod deletes;
mod devices;
mod embeddings;
mod events;
mod message_citations;
mod messages;
mod notes;
mod sessions;
mod timestamps;
mod tokens;
mod users;

pub use ai_jobs::{
    AiJob, AiJobFilter, AiJobKind, AiJobKindFilter, AiJobStatus, AiJobsRepo, WikiLintScope,
};
pub use assets::{AssetRow, AssetsRepo};
pub use chunks::ChunksRepo;
pub use conversations::{ConversationsRepo, ListConversationsParams};
pub use cursors::DeviceCursorRepo;
pub use deletes::{DeleteRecordRow, DeleteRecordsRepo};
pub use devices::{DeviceRepo, DeviceRow};
pub use embeddings::{EmbeddingsRepo, TopHit};
pub use events::{NewSyncEvent, SyncEventRow, SyncEventsRepo};
pub use message_citations::MessageCitationsRepo;
pub use messages::{AppendMessageParams, ListMessagesParams, MessagesRepo};
pub use notes::{NoteRow, NotesRepo};
pub use sessions::{WebSessionInsert, WebSessionRepo, WebSessionRow};
pub use tokens::{DeviceTokenInsert, DeviceTokenRepo, DeviceTokenRow};
pub use users::{UserPasswordRow, UserRepo, UserRow};
pub use zk_domain::{decode_vector, encode_vector};
