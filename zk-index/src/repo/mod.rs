mod assets;
mod chunks;
mod delete_records;
mod devices;
mod embeddings;
mod links;
mod notes;
mod sources;
mod sync_state;

pub use assets::AssetRepo;
// local and server schemas use different parent-note tables and error types.
pub use chunks::ChunksRepo;
pub use delete_records::DeleteRecordRepo;
pub use devices::DeviceRepo;
pub use embeddings::{EmbeddingsRepo, TopHit};
pub use links::LinkRepo;
pub use notes::{ListedNote, NoteIndexedMetadata, NoteListQuery, NoteRepo, UpsertNoteParams};
pub use sources::SourcesRepo;
pub use sync_state::{SyncStateRepo, SyncedAssetStateInput, SyncedNoteStateInput};
