INCLUDE(BuildPlugin)
include_directories(SYSTEM ${FFTW3F_INCLUDE_DIRS})
LINK_LIBRARIES(${FFTW3F_LIBRARIES})

BUILD_PLUGIN(ampandir AmpAndIr.cpp AmpAndIrControls.cpp Convolver.cpp
	MOCFILES AmpAndIrControls.h)
