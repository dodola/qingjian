cmake_minimum_required(VERSION 3.16)

project(fcitx5-qingjian VERSION 0.1.0)

set(CMAKE_EXPORT_COMPILE_COMMANDS ON)
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

find_package(ECM REQUIRED)
set(CMAKE_MODULE_PATH ${ECM_MODULE_PATH} ${CMAKE_CURRENT_SOURCE_DIR}/cmake)

find_package(Fcitx5Core 5.0 REQUIRED)

include(FeatureSummary)
include(GNUInstallDirs)
include(ECMSetupVersion)
include(ECMUninstallTarget)

include("${FCITX_INSTALL_CMAKECONFIG_DIR}/Fcitx5Utils/Fcitx5CompilerSettings.cmake")

# Rust cdylib 由 cargo 构建，addon 链接它（与 karukan / Bonolith 同做法）。
set(QINGJIAN_RUST_LIB
    "${CMAKE_CURRENT_SOURCE_DIR}/../../../target/release/libqingjian_fcitx5.so")

find_program(CARGO cargo REQUIRED)

add_custom_target(qingjian_rust_lib ALL
    COMMAND ${CARGO} build --release -p qingjian-fcitx5
    WORKING_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}/../../.."
    COMMENT "Building qingjian-fcitx5 Rust library"
    BYPRODUCTS "${QINGJIAN_RUST_LIB}"
)

add_library(qingjian MODULE src/qingjian.cpp)
add_dependencies(qingjian qingjian_rust_lib)

target_include_directories(qingjian PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/src)

# 候选注释（CandidateWord::setComment）是 fcitx5 5.1.9 起才有的。
if(Fcitx5Core_VERSION VERSION_GREATER_EQUAL "5.1.9")
    target_compile_definitions(qingjian PRIVATE FCITX5_HAS_CANDIDATE_SET_COMMENT)
endif()

target_link_libraries(qingjian
    Fcitx5::Core
    Fcitx5::Config
    ${QINGJIAN_RUST_LIB}
)

# fcitx5 期望 addon 是 `qingjian.so` 而不是 `libqingjian.so`。
set_target_properties(qingjian PROPERTIES PREFIX "")

# 运行时找同目录的 libqingjian_fcitx5.so（安装后 $ORIGIN 就是 addondir）。
set_target_properties(qingjian PROPERTIES
    BUILD_RPATH "${CMAKE_CURRENT_SOURCE_DIR}/../../../target/release"
    INSTALL_RPATH "\$ORIGIN"
    BUILD_WITH_INSTALL_RPATH FALSE
)

install(TARGETS qingjian DESTINATION "${FCITX_INSTALL_ADDONDIR}")
install(FILES "${QINGJIAN_RUST_LIB}" DESTINATION "${FCITX_INSTALL_ADDONDIR}")

configure_file(qingjian-addon.conf.in
    "${CMAKE_CURRENT_BINARY_DIR}/qingjian-addon.conf" @ONLY)
install(FILES "${CMAKE_CURRENT_BINARY_DIR}/qingjian-addon.conf"
    DESTINATION "${FCITX_INSTALL_PKGDATADIR}/addon"
    RENAME qingjian.conf)

install(FILES qingjian.conf
    DESTINATION "${FCITX_INSTALL_PKGDATADIR}/inputmethod")

configure_file(org.fcitx.Fcitx5.Addon.Qingjian.metainfo.xml.in
    "${CMAKE_CURRENT_BINARY_DIR}/org.fcitx.Fcitx5.Addon.Qingjian.metainfo.xml" @ONLY)
install(FILES "${CMAKE_CURRENT_BINARY_DIR}/org.fcitx.Fcitx5.Addon.Qingjian.metainfo.xml"
    DESTINATION "${CMAKE_INSTALL_DATAROOTDIR}/metainfo")

install(FILES icons/fcitx-qingjian-16.png
    DESTINATION "${CMAKE_INSTALL_DATAROOTDIR}/icons/hicolor/16x16/apps/"
    RENAME fcitx-qingjian.png)
install(FILES icons/fcitx-qingjian-24.png
    DESTINATION "${CMAKE_INSTALL_DATAROOTDIR}/icons/hicolor/24x24/apps/"
    RENAME fcitx-qingjian.png)
install(FILES icons/fcitx-qingjian-32.png
    DESTINATION "${CMAKE_INSTALL_DATAROOTDIR}/icons/hicolor/32x32/apps/"
    RENAME fcitx-qingjian.png)
install(FILES icons/fcitx-qingjian-48.png
    DESTINATION "${CMAKE_INSTALL_DATAROOTDIR}/icons/hicolor/48x48/apps/"
    RENAME fcitx-qingjian.png)
install(FILES icons/fcitx-qingjian-128.png
    DESTINATION "${CMAKE_INSTALL_DATAROOTDIR}/icons/hicolor/128x128/apps/"
    RENAME fcitx-qingjian.png)

feature_summary(WHAT ALL FATAL_ON_MISSING_REQUIRED_PACKAGES)