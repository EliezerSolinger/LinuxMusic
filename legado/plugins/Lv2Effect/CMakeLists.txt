IF(LMMS_HAVE_LV2)
	include_directories(SYSTEM ${LV2_INCLUDE_DIRS})
	include_directories(SYSTEM ${Lilv_INCLUDE_DIRS})
	include_directories(SYSTEM ${SUIL_INCLUDE_DIRS})
	INCLUDE(BuildPlugin)
	BUILD_PLUGIN(lv2effect Lv2Effect.cpp Lv2FxControls.cpp Lv2FxControlDialog.cpp Lv2Effect.h Lv2FxControls.h Lv2FxControlDialog.h
		MOCFILES Lv2Effect.h Lv2FxControls.h Lv2FxControlDialog.h
		EMBEDDED_RESOURCES logo.png)

	# The process that runs one effect isolated, when the user (or the PluginGuard, for a plugin that crashed)
	# asks for it
	add_executable(RemoteLv2Host
		RemoteLv2Host.cpp
		"${LMMS_SOURCE_DIR}/src/common/RemotePluginBase.cpp"
		"${LMMS_SOURCE_DIR}/src/common/SharedMemory.cpp"
		"${LMMS_SOURCE_DIR}/src/common/SystemSemaphore.cpp"
	)
	target_compile_definitions(RemoteLv2Host PRIVATE BUILD_REMOTE_PLUGIN_CLIENT)
	target_include_directories(RemoteLv2Host PRIVATE "${LMMS_SOURCE_DIR}/include" "${CMAKE_BINARY_DIR}")
	target_compile_features(RemoteLv2Host PRIVATE cxx_std_20)
	find_package(Threads REQUIRED)
	target_link_libraries(RemoteLv2Host PRIVATE Lilv::lilv Threads::Threads)
	INSTALL(TARGETS RemoteLv2Host RUNTIME DESTINATION "${PLUGIN_DIR}")
	SET_PROPERTY(GLOBAL APPEND PROPERTY PLUGINS_BUILT "RemoteLv2Host")
	add_dependencies(lv2effect RemoteLv2Host)
ENDIF(LMMS_HAVE_LV2)
