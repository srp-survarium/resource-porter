# Sourced by bake.sh and bake-pack.sh after `shift`: `blender` runs BLENDER
# (a command, `blender` by default) at nice 19, holding the GPU lock
# (LOBBY_BAKE_GPU_LOCK) when the options ("$@") ask for --gpu; `show` holds
# the options show.py takes from them.
gpu=0
show=()
for a in "$@"; do [[ $a == --gpu ]] && gpu=1 && show=(--gpu); done
blender() {
    if ((gpu)); then
        flock "${LOBBY_BAKE_GPU_LOCK:-/mnt/hdd/Projects/lobby-bake/gpu.lock}" nice -n 19 ${BLENDER:-blender} "$@"
    else
        nice -n 19 ${BLENDER:-blender} "$@"
    fi
}
