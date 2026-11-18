include(CTest)

set(CMAKE_INCLUDE_CURRENT_DIR ON)
set(CMAKE_AUTOMOC ON)

set(LMMS_TESTS
	src/core/AmpModelTest.cpp
	src/core/ArpeggioSyncTest.cpp
	src/core/AtomicPeakTest.cpp
	src/core/ArrayVectorTest.cpp
	src/core/ConvolverTest.cpp
	src/core/DrawnWaveTest.cpp
	src/core/DrumSynthTest.cpp
	src/core/EditWhilePlayingTest.cpp
	src/core/PeakControllerTest.cpp
	src/core/PluginCategoriesTest.cpp
	src/core/InstrumentFixesTest.cpp
	src/core/EngineLifecycleTest.cpp
	src/core/ErrorLogTest.cpp
	src/core/DeviceWaitTest.cpp
	src/core/AudioBufferTest.cpp
	src/core/AudioInputTest.cpp
	src/core/AudioThreadAllocationTest.cpp
	src/core/AudioTrackTest.cpp
	src/core/AudioPortsTest.cpp
	src/core/AutomatableModelTest.cpp
	src/core/AutosaveTest.cpp
	src/core/BuiltinPluginsTest.cpp
	src/core/CompressedProjectTest.cpp
	src/core/ConfigManagerTest.cpp
	src/core/ExportProcessorTest.cpp
	src/core/GainStagingTest.cpp
	src/core/ControllerConnectionTest.cpp
	src/core/LadspaIsolationTest.cpp
	src/core/Lv2IsolationTest.cpp
	src/core/LfoControllerTest.cpp
	src/core/LoudnessReferenceTest.cpp
	src/core/MidiByteParserTest.cpp
	src/core/MidiAppleTest.cpp
	src/core/LoadCorruptedTest.cpp
	src/core/Lv2WorkerTest.cpp
	src/core/MathTest.cpp
	src/core/MetronomeTest.cpp
	src/core/MidiClientChangeTest.cpp
	src/core/MidiTempoMapTest.cpp
	src/core/MixerTest.cpp
	src/core/PluginEventTest.cpp
	src/core/PluginGuardTest.cpp
	src/core/ProjectMediaTest.cpp
	src/core/ProjectStateTest.cpp
	src/core/ProjectVersionTest.cpp
	src/core/RealtimeAllocatorTest.cpp
	src/core/RecordingTest.cpp
	src/core/SingleStreamOrderTest.cpp
	src/core/RemotePluginHangTest.cpp
	src/core/RemotePluginIpcTest.cpp
	src/core/RelativePathsTest.cpp
	src/core/SongMarkersTest.cpp
	src/core/SpscRingTest.cpp
	src/core/StreamingTest.cpp
	src/core/TempoMapTest.cpp
	src/core/ThreadedExportTest.cpp
	src/core/ThreadStressTest.cpp
	src/core/TimelineTest.cpp
	src/core/TimeStretcherTest.cpp
	src/core/TransportTest.cpp
	src/core/TripleBufferTest.cpp
	src/core/UndoCommandTest.cpp
	src/core/UserNoticeTest.cpp
	src/gui/SampleThumbnailRendererTest.cpp
	src/gui/SetupDialogTest.cpp
	src/gui/ShortcutsTest.cpp
	src/gui/ThemeImagesTest.cpp
	src/gui/TransportBarTest.cpp
	src/tracks/AutomationTrackTest.cpp
	src/tracks/SampleTrackTest.cpp
)

foreach(LMMS_TEST_SRC IN LISTS LMMS_TESTS)
	# TODO CMake 3.20: Use cmake_path
	get_filename_component(LMMS_TEST_NAME ${LMMS_TEST_SRC} NAME_WE)

	add_executable(${LMMS_TEST_NAME} ${LMMS_TEST_SRC} src/TestEnvironment.cpp)
	# The program's operator new and delete, which the plugins (linked against the program) take from it; the
	# allocation test has counting ones of its own
	if(NOT LMMS_TEST_NAME STREQUAL "AudioThreadAllocationTest")
		target_sources(${LMMS_TEST_NAME} PRIVATE ${CMAKE_SOURCE_DIR}/src/core/RealtimeNew.cpp)
	endif()
	add_test(NAME ${LMMS_TEST_NAME} COMMAND ${LMMS_TEST_NAME})
	if(WANT_DEBUG_TSAN)
		# (what ThreadSanitizer cannot see through Qt, see tsan.supp)
		set_tests_properties(${LMMS_TEST_NAME} PROPERTIES
			ENVIRONMENT "TSAN_OPTIONS=suppressions=${CMAKE_CURRENT_SOURCE_DIR}/tsan.supp")
	endif()
	target_compile_definitions(${LMMS_TEST_NAME} PRIVATE LMMS_TEST_PLUGIN_DIR="${CMAKE_BINARY_DIR}/plugins"
		LMMS_TEST_DATA_DIR="${CMAKE_SOURCE_DIR}/data/")

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

# A plugin process for RemotePluginHangTest, speaking the same protocol as the real ones (next to the test, where
# RemotePlugin looks for it first on macOS; elsewhere through LMMS_PLUGIN_DIR)
add_executable(FakeRemotePlugin
	src/helpers/FakeRemotePlugin.cpp
	${CMAKE_SOURCE_DIR}/src/common/RemotePluginBase.cpp
	${CMAKE_SOURCE_DIR}/src/common/SharedMemory.cpp
	${CMAKE_SOURCE_DIR}/src/common/SystemSemaphore.cpp
)
target_compile_definitions(FakeRemotePlugin PRIVATE BUILD_REMOTE_PLUGIN_CLIENT)
target_include_directories(FakeRemotePlugin PRIVATE ${CMAKE_SOURCE_DIR}/include ${CMAKE_BINARY_DIR})
target_compile_features(FakeRemotePlugin PRIVATE cxx_std_20)
find_package(Threads REQUIRED)
target_link_libraries(FakeRemotePlugin PRIVATE Threads::Threads)
add_dependencies(RemotePluginHangTest FakeRemotePlugin)
set_tests_properties(RemotePluginHangTest PROPERTIES ENVIRONMENT "LMMS_PLUGIN_DIR=${CMAKE_CURRENT_BINARY_DIR}")

# A LADSPA plugin that crashes on demand, for LadspaIsolationTest (in a folder of its own, given in LADSPA_PATH)
add_library(CrashingLadspa MODULE src/helpers/CrashingLadspa.c)
target_include_directories(CrashingLadspa PRIVATE ${CMAKE_SOURCE_DIR}/include)
set_target_properties(CrashingLadspa PROPERTIES PREFIX "" SUFFIX ".so"
	LIBRARY_OUTPUT_DIRECTORY "${CMAKE_CURRENT_BINARY_DIR}/ladspa-test")
if(WIN32)
	set_target_properties(CrashingLadspa PROPERTIES SUFFIX ".dll")
endif()
add_dependencies(LadspaIsolationTest CrashingLadspa)
target_compile_definitions(LadspaIsolationTest PRIVATE LMMS_TEST_LADSPA_DIR="${CMAKE_CURRENT_BINARY_DIR}/ladspa-test")

# An LV2 plugin for Lv2IsolationTest (a bundle in a folder of its own, given in LV2_PATH): gain through the worker, a CV
# input, a latency, an event input, and a crash on demand
if(LMMS_HAVE_LV2)
	set(TEST_LV2_BUNDLE "${CMAKE_CURRENT_BINARY_DIR}/lv2-test/linuxmusic-test.lv2")
	add_library(linuxmusic_test_lv2 MODULE src/helpers/TestLv2/plugin.c)
	target_include_directories(linuxmusic_test_lv2 SYSTEM PRIVATE ${LV2_INCLUDE_DIRS})
	set_target_properties(linuxmusic_test_lv2 PROPERTIES PREFIX "" SUFFIX ".so" LIBRARY_OUTPUT_DIRECTORY "${TEST_LV2_BUNDLE}")
	configure_file(src/helpers/TestLv2/manifest.ttl "${TEST_LV2_BUNDLE}/manifest.ttl" COPYONLY)
	configure_file(src/helpers/TestLv2/plugin.ttl "${TEST_LV2_BUNDLE}/plugin.ttl" COPYONLY)
	add_dependencies(Lv2IsolationTest linuxmusic_test_lv2)
	target_compile_definitions(Lv2IsolationTest PRIVATE LMMS_TEST_LV2_DIR="${CMAKE_CURRENT_BINARY_DIR}/lv2-test")
endif()

# The damaged projects are made from the A/B projects
target_compile_definitions(LoadCorruptedTest PRIVATE LMMS_TEST_AB_PROJECTS="${CMAKE_SOURCE_DIR}/tools/check/ab-projects")

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

# The system allocator's calls, seen by AudioThreadAllocationTest (Qt and C code call malloc themselves)
if(APPLE OR CMAKE_SYSTEM_NAME STREQUAL "Linux")
	add_library(AllocationObserver SHARED src/support/AllocationObserver.c)
	add_dependencies(AudioThreadAllocationTest AllocationObserver)
	if(APPLE)
		set_tests_properties(AudioThreadAllocationTest PROPERTIES
			ENVIRONMENT "DYLD_INSERT_LIBRARIES=$<TARGET_FILE:AllocationObserver>")
	else()
		set_tests_properties(AudioThreadAllocationTest PROPERTIES
			ENVIRONMENT "LD_PRELOAD=$<TARGET_FILE:AllocationObserver>")
	endif()
endif()

# The scripted check that the core does not include the GUI (see scripted/check-core-gui)
find_package(Python3 COMPONENTS Interpreter)
if(Python3_Interpreter_FOUND)
	add_test(NAME check-core-gui COMMAND ${Python3_EXECUTABLE} ${CMAKE_CURRENT_SOURCE_DIR}/scripted/check-core-gui)
endif()
