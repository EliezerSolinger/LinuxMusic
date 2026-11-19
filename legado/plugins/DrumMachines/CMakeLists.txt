include(BuildPlugin)
# The machines' photos: photos/CREDITS.md
build_plugin(drummachines DrumMachines.cpp DrumMachines.h MOCFILES DrumMachines.h EMBEDDED_RESOURCES photos/*.jpg)
