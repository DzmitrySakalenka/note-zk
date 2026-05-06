// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::any::{type_name, type_name_of_val};

use zk_server::ai::{
    ingest::IngestEnqueuer,
    worker::{AiWorkerDeps, spawn_ai_job_worker},
};

#[test]
fn server_ai_module_is_available_from_crate_root() {
    assert!(!type_name::<IngestEnqueuer>().is_empty());
    assert!(!type_name::<AiWorkerDeps>().is_empty());

    assert!(type_name_of_val(&spawn_ai_job_worker).contains("spawn_ai_job_worker"));
}
