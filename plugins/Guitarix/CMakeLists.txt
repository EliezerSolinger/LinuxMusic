INCLUDE(BuildPlugin)

# Guitarix's own files (gx/), as they are in Guitarix: its units' DSP, the headers they are written against and the
# zita-resampler they run at 96 kHz with (GPL-3.0 or later: see gx/COPYING and gx/zita-resampler/README)
SET(GUITARIX_UNITS
	gx/plugins/ts9sim.cc
	gx/plugins/aclipper.cc
	gx/plugins/mxrdist.cc
	gx/plugins/bossds1.cc
	gx/plugins/bmp.cc
	gx/plugins/flanger_gx.cc
	gx/plugins/mbc.cc
	gx/plugins/mbd.cc
	gx/plugins/mbe.cc
	gx/plugins/mbdel.cc
	gx/plugins/mbclipper.cc
	gx/plugins/bassboom.cc
	gx/plugins/mbchor.cc
	gx/plugins/gcb_95.cc
	gx/plugins/fuzzfacefm.cc
	gx/plugins/fuzzfacerm.cc
	gx/plugins/hornet.cc
	gx/plugins/susta.cc
	gx/plugins/hfb.cc
	gx/plugins/muff.cc
	gx/plugins/scream.cc
	gx/plugins/lpbboost.cc
	gx/plugins/foxeylady.cc
	gx/plugins/hogsfoot.cc
	gx/plugins/cstb.cc
	gx/plugins/coverd.cc
	gx/plugins/rangem.cc
	gx/plugins/ruin.cc
	gx/plugins/ffreak.cc
	gx/plugins/fumaster.cc
	gx/plugins/fuzzdrive.cc
	gx/plugins/rolandwah.cc
	gx/plugins/astrofuzz.cc
	gx/plugins/buffb.cc
	gx/plugins/eldist.cc
	gx/plugins/mole.cc
	gx/plugins/buzz.cc
	gx/plugins/bfuzz.cc
	gx/plugins/axface.cc
	gx/plugins/mbcs.cc
	gx/plugins/zita_rev1.cc
)
SET(GUITARIX_SUPPORT
	gx/gx_resampler.cpp
	gx/zita-resampler/resampler.cc
	gx/zita-resampler/resampler-table.cc
)

BUILD_PLUGIN(guitarix
	GuitarixEffect.cpp GuitarixUnits.cpp GuitarixControlDialog.cpp
	${GUITARIX_UNITS} ${GUITARIX_SUPPORT}
	MOCFILES GuitarixEffect.h GuitarixControlDialog.h
)
TARGET_INCLUDE_DIRECTORIES(guitarix PRIVATE
	"${CMAKE_CURRENT_SOURCE_DIR}/gx/headers"
	"${CMAKE_CURRENT_SOURCE_DIR}/gx/plugins"
	"${CMAKE_CURRENT_SOURCE_DIR}/gx/zita-resampler"
)
# (Guitarix's code as it is: its own warnings are not ours to fix)
SET_SOURCE_FILES_PROPERTIES(${GUITARIX_UNITS} ${GUITARIX_SUPPORT} PROPERTIES COMPILE_OPTIONS "-w")
