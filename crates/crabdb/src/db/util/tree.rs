use super::*;

pub(crate) fn tree_root_hex(tree: &Tree) -> Option<String> {
    tree.root.as_ref().map(|cid| hex::encode(cid.as_bytes()))
}

pub(crate) fn tree_from_root_hex(root: Option<&str>) -> Result<Tree> {
    tree_from_root_hex_with_config(root, prolly_config())
}

pub(crate) fn worktree_root_map_tree_from_root_hex(root: Option<&str>) -> Result<Tree> {
    tree_from_root_hex_with_config(root, worktree_root_map_prolly_config())
}

fn tree_from_root_hex_with_config(root: Option<&str>, config: Config) -> Result<Tree> {
    let cid = match root {
        Some(hex_root) => {
            let bytes = hex::decode(hex_root)
                .map_err(|err| Error::Corrupt(format!("invalid tree root hex: {err}")))?;
            let bytes: [u8; 32] = bytes
                .as_slice()
                .try_into()
                .map_err(|_| Error::Corrupt("tree root CID must be 32 bytes".to_string()))?;
            Some(Cid(bytes))
        }
        None => None,
    };
    Ok(Tree { root: cid, config })
}
