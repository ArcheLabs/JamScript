# Managed-state PolkaVM probe

This probe exercises `ProofState`, a first trie read, transactional writes,
and root finalization in a real guest. It is intentionally separate from
application/game fixtures so authenticated state failures can be isolated.
