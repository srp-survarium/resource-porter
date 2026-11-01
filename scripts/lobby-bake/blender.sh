# Sourced by bake.sh and bake-pack.sh after `shift`: `blender` runs BLENDER
# (a command, `blender` by default) at nice 19; with --gpu among the options
# ("$@"), on the lock LOBBY_BAKE_GPU_LOCK when it is set (a BLENDER command
# that takes the GPU lock itself, as blender-official/run-blender.sh, needs
# none); `show` holds the options show.py takes from them.
show=()
gpu=0
args=("$@")
for ((i = 0; i < ${#args[@]}; i++)); do
    case ${args[i]} in
        --gpu) gpu=1; show+=(--gpu) ;;
        --fov | --threads | --exposure | --match | --bulb | --skip-lights | --skip-props) show+=("${args[i]}" "${args[i + 1]}") ;;
    esac
done
blender() {
    if ((gpu)) && [[ -n ${LOBBY_BAKE_GPU_LOCK:-} ]]; then
        flock "$LOBBY_BAKE_GPU_LOCK" nice -n 19 ${BLENDER:-blender} "$@"
    else
        nice -n 19 ${BLENDER:-blender} "$@"
    fi
}
