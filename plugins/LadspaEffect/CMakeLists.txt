INCLUDE(BuildPlugin)

BUILD_PLUGIN(ladspaeffect LadspaEffect.cpp LadspaControls.cpp LadspaControlDialog.cpp LadspaMatrixControlDialog.cpp LadspaSubPluginFeatures.cpp LadspaWidgetFactory.cpp LadspaEffect.h LadspaControls.h LadspaControlDialog.h LadspaMatrixControlDialog.h LadspaSubPluginFeatures.h LadspaWidgetFactory.h MOCFILES LadspaEffect.h LadspaControls.h LadspaControlDialog.h LadspaMatrixControlDialog.h EMBEDDED_RESOURCES logo.png)

# The process that runs one effect isolated, when the user (or the PluginGuard, for a plugin that crashed) asks for it
add_executable(RemoteLadspaHost
	RemoteLadspaHost.cpp
	"${LMMS_SOURCE_DIR}/src/common/RemotePluginBase.cpp"
	"${LMMS_SOURCE_DIR}/src/common/SharedMemory.cpp"
	"${LMMS_SOURCE_DIR}/src/common/SystemSemaphore.cpp"
)
target_compile_definitions(RemoteLadspaHost PRIVATE BUILD_REMOTE_PLUGIN_CLIENT)
target_include_directories(RemoteLadspaHost PRIVATE "${LMMS_SOURCE_DIR}/include" "${CMAKE_BINARY_DIR}")
target_compile_features(RemoteLadspaHost PRIVATE cxx_std_20)
find_package(Threads REQUIRED)
target_link_libraries(RemoteLadspaHost PRIVATE ${CMAKE_DL_LIBS} Threads::Threads)
INSTALL(TARGETS RemoteLadspaHost RUNTIME DESTINATION "${PLUGIN_DIR}")
SET_PROPERTY(GLOBAL APPEND PROPERTY PLUGINS_BUILT "RemoteLadspaHost")
add_dependencies(ladspaeffect RemoteLadspaHost)

SET(CMAKE_LIBRARY_OUTPUT_DIRECTORY "${CMAKE_LIBRARY_OUTPUT_DIRECTORY}/ladspa")

# TODO: Plugins themselves should use __attribute__((visibility("default"))) to export "ladspa_descriptor"
set(CMAKE_C_VISIBILITY_PRESET default)
set(CMAKE_CXX_VISIBILITY_PRESET default)

IF(LMMS_HAVE_CAPS)
ADD_SUBDIRECTORY(caps)
ENDIF()

IF(LMMS_HAVE_TAP)
ADD_SUBDIRECTORY(tap)
ENDIF()

IF(LMMS_HAVE_SWH)
ADD_SUBDIRECTORY(swh)
ENDIF()

IF(LMMS_HAVE_CMT)
ADD_SUBDIRECTORY(cmt)
ENDIF()

IF(LMMS_HAVE_CALF)
ADD_SUBDIRECTORY(calf)
ENDIF()
