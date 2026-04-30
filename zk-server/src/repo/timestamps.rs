use std::io::{self, ErrorKind};

use chrono::{DateTime, Utc};
use rusqlite::{Row, types::Type};

pub(super) fn encode_timestamp(timestamp: DateTime<Utc>) -> String {
    timestamp.to_rfc3339()
}

pub(super) fn decode_timestamp(row: &Row<'_>, index: usize) -> rusqlite::Result<DateTime<Utc>> {
    let raw_timestamp = row.get::<_, String>(index)?;

    DateTime::parse_from_rfc3339(&raw_timestamp)
        .map(|parsed| parsed.with_timezone(&Utc))
        .map_err(|source| {
            rusqlite::Error::FromSqlConversionFailure(
                index,
                Type::Text,
                Box::new(io::Error::new(ErrorKind::InvalidData, source.to_string())),
            )
        })
}

pub(super) fn decode_optional_timestamp(
    row: &Row<'_>,
    index: usize,
) -> rusqlite::Result<Option<DateTime<Utc>>> {
    let raw_timestamp = row.get::<_, Option<String>>(index)?;

    raw_timestamp
        .map(|value| {
            DateTime::parse_from_rfc3339(&value)
                .map(|parsed| parsed.with_timezone(&Utc))
                .map_err(|source| {
                    rusqlite::Error::FromSqlConversionFailure(
                        index,
                        Type::Text,
                        Box::new(io::Error::new(ErrorKind::InvalidData, source.to_string())),
                    )
                })
        })
        .transpose()
}
