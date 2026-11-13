include(CTest)

set(CMAKE_INCLUDE_CURRENT_DIR ON)
set(CMAKE_AUTOMOC ON)

set(LMMS_TESTS
	src/core/AmpModelTest.cpp
	src/core/ArrayVectorTest.cpp
	src/core/ConvolverTest.cpp
	src/core/AudioBufferTest.cpp
	src/core/AudioInputTest.cpp
	src/core/AudioPortsTest.cpp
	src/core/AutomatableModelTest.cpp
	src/core/ExportProcessorTest.cpp
	src/core/GainStagingTest.cpp
	src/core/MathTest.cpp
	src/core/MixerTest.cpp
	src/core/PluginEventTest.cpp
	src/core/ProjectMediaTest.cpp
	src/core/ProjectVersionTest.cpp
	src/core/RecordingTest.cpp
	src/core/RelativePathsTest.cpp
	src/core/SpscRingTest.cpp
	src/core/TempoMapTest.cpp
	src/core/TimelineTest.cpp
	src/core/TimeStretcherTest.cpp
	src/core/TripleBufferTest.cpp
	src/gui/ThemeImagesTest.cpp
	src/tracks/AutomationTrackTest.cpp
)

foreach(LMMS_TEST_SRC IN LISTS LMMS_TESTS)
	# TODO CMake 3.20: Use cmake_path
	get_filename_component(LMMS_TEST_NAME ${LMMS_TEST_SRC} NAME_WE)

	add_executable(${LMMS_TEST_NAME} ${LMMS_TEST_SRC} src/TestEnvironment.cpp)
	add_test(NAME ${LMMS_TEST_NAME} COMMAND ${LMMS_TEST_NAME})

	# TODO CMake 3.12: Propagate usage requirements by linking to lmmsobjs
	target_include_directories(${LMMS_TEST_NAME} PRIVATE $<TARGET_PROPERTY:lmmsobjs,INCLUDE_DIRECTORIES>)

	target_static_libraries("${LMMS_TEST_NAME}" PRIVATE lmmsobjs)
	target_link_libraries(${LMMS_TEST_NAME} PRIVATE
		${QT_LIBRARIES}
		${QT_QTTEST_LIBRARY}
	)

	target_compile_features(${LMMS_TEST_NAME} PRIVATE cxx_std_20)
	target_compile_definitions(${LMMS_TEST_NAME} PRIVATE LMMS_TESTING)
endforeach()

# The Amp & IR plugin's convolver, tested on its own
target_sources(ConvolverTest PRIVATE ${CMAKE_SOURCE_DIR}/plugins/AmpAndIr/Convolver.cpp)
target_include_directories(ConvolverTest PRIVATE ${CMAKE_SOURCE_DIR}/plugins/AmpAndIr ${FFTW3F_INCLUDE_DIRS})
target_link_libraries(ConvolverTest PRIVATE ${FFTW3F_LIBRARIES})

# The Amp & IR plugin's amp model, tested on its own with Neural Amp Modeler's example models
target_sources(AmpModelTest PRIVATE ${CMAKE_SOURCE_DIR}/plugins/AmpAndIr/AmpModel.cpp)
target_include_directories(AmpModelTest PRIVATE ${CMAKE_SOURCE_DIR}/plugins/AmpAndIr)
target_link_libraries(AmpModelTest PRIVATE nam_core ${SAMPLERATE_LIBRARIES})
target_compile_definitions(AmpModelTest PRIVATE
	AMP_MODEL_EXAMPLES="${CMAKE_SOURCE_DIR}/plugins/AmpAndIr/NeuralAmpModelerCore/example_models")
