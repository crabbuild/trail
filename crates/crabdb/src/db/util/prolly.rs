use super::*;

pub(crate) fn prolly_config() -> Config {
    Config::builder()
        .min_chunk_size(4)
        .max_chunk_size(1024)
        .chunking_factor(128)
        .hash_seed(0xC0DB)
        .encoding(Encoding::Raw)
        .build()
}
