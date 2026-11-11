# "Send to phone" in KDE's Share menu (Purpose: Dolphin, Spectacle, Gwenview, Okular...). Purpose
# only loads compiled plugins, so this is a small one: it hands the files to
# `vyoma-desktop --send-any`, which sends them to the connected phone or asks which.
#   cmake -B build -G Ninja && cmake --build build && cmake --install build --prefix ~/.local
cmake_minimum_required(VERSION 3.16)
project(vyoma-purpose LANGUAGES CXX)

find_package(ECM 6.0 REQUIRED NO_MODULE)
set(CMAKE_MODULE_PATH ${ECM_MODULE_PATH})
include(KDEInstallDirs)
include(KDECMakeSettings)
include(KDECompilerSettings NO_POLICY_SCOPE)

find_package(Qt6 6.5 REQUIRED COMPONENTS Core Gui)
find_package(KF6 6.0 REQUIRED COMPONENTS CoreAddons Purpose)

kcoreaddons_add_plugin(vyomaplugin SOURCES vyomaplugin.cpp INSTALL_NAMESPACE "kf6/purpose")
target_link_libraries(vyomaplugin Qt6::Core KF6::CoreAddons KF6::Purpose)
