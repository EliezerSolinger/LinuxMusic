INCLUDE(BuildPlugin)
include_directories(SYSTEM ${FFTW3F_INCLUDE_DIRS})
LINK_LIBRARIES(${FFTW3F_LIBRARIES})

# Neural Amp Modeler's core (MIT), with Eigen (MPL 2.0) and nlohmann/json (MIT): third-party code, built apart
set(NAM_DIR "${CMAKE_CURRENT_SOURCE_DIR}/NeuralAmpModelerCore")
# (an object library: its architectures register themselves from static objects, which a static library would drop)
file(GLOB_RECURSE NAM_SOURCES "${NAM_DIR}/NAM/*.cpp")
add_library(nam_core OBJECT ${NAM_SOURCES})
set_target_properties(nam_core PROPERTIES SYSTEM TRUE POSITION_INDEPENDENT_CODE ON)
target_compile_features(nam_core PUBLIC cxx_std_20)
target_compile_definitions(nam_core PUBLIC NAM_SAMPLE_FLOAT)
target_include_directories(nam_core SYSTEM PUBLIC "${NAM_DIR}" "${NAM_DIR}/Dependencies/eigen"
	"${NAM_DIR}/Dependencies/nlohmann")

BUILD_PLUGIN(ampandir AmpAndIr.cpp AmpAndIrControls.cpp Convolver.cpp AmpModel.cpp
	MOCFILES AmpAndIrControls.h)
target_link_libraries(ampandir nam_core)
