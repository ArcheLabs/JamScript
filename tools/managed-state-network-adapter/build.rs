fn main() {
    println!("cargo:rerun-if-changed=../../examples/game-replay/native/game/replay.c");
    println!("cargo:rerun-if-changed=../../examples/game-replay/native/game/replay.h");
    cc::Build::new()
        .file("../../examples/game-replay/native/game/replay.c")
        .include("../../examples/game-replay/native/game")
        .compile("jamscript_game_replay_host");
}
