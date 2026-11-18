INCLUDE(BuildPlugin)

# Guitarix's own files (gx/), as they are in Guitarix: its units' DSP, the headers they are written against and the
# zita-resampler they run at 96 kHz with (GPL-3.0 or later: see gx/COPYING and gx/zita-resampler/README)
SET(GUITARIX_UNITS
	gx/plugins/ts9sim.cc
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
