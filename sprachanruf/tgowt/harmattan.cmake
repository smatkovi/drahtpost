# CMake-Werkzeugkette: tg_owt fuer MeeGo Harmattan (armv7, hart, NEON).
#
# Der Uebersetzer ist das clang aus dem WebRTC-Abzug, nicht das des
# Baurechners -- mitsamt dessen libc++. Das ist kein Geschmack: die
# libstdc++ im Sysroot ist GCC 4.4 von 2009 und kann kein C++17. WebRTC
# bringt seine eigene libc++ mit, und die laeuft ohne Systemunterstuetzung.
#
# WEBRTC_SRC (der src-Baum) und SCHICHT (die Kompatibilitaetsschicht der
# elf Luecken von 2009) muessen von aussen gesetzt sein.

set(CMAKE_SYSTEM_NAME Linux)
set(CMAKE_SYSTEM_PROCESSOR arm)

# Die Werkzeugkettendatei wird auch fuer die Probeuebersetzungen von
# CMake noch einmal gelesen, und zwar in einem eigenen Projekt ohne
# unseren Zwischenspeicher. Ohne diese Zeile stuenden dort WEBRTC_SRC und
# die anderen leer.
list(APPEND CMAKE_TRY_COMPILE_PLATFORM_VARIABLES WEBRTC_SRC SCHICHT SYSROOT)

if (NOT DEFINED WEBRTC_SRC OR NOT DEFINED SCHICHT OR NOT DEFINED SYSROOT)
    message(FATAL_ERROR "WEBRTC_SRC, SCHICHT und SYSROOT muessen gesetzt sein.")
endif()

set(LLVM ${WEBRTC_SRC}/third_party/llvm-build/Release+Asserts/bin)
set(CMAKE_C_COMPILER   ${LLVM}/clang)
set(CMAKE_CXX_COMPILER ${LLVM}/clang++)
set(CMAKE_AR           ${LLVM}/llvm-ar     CACHE FILEPATH "")
# llvm-ranlib bringt der WebRTC-Abzug nicht mit -- llvm-ar schreibt das
# Symbolverzeichnis ohnehin selbst; das des Baurechners genuegt.
find_program(CMAKE_RANLIB NAMES llvm-ranlib ranlib REQUIRED)

# Beim Einrichten nur uebersetzen, nicht binden: ein vollstaendiges
# Programm braeuchte die Anlaufdateien des Sysroots, und die Pruefungen in
# arch.cmake fragen ohnehin nur den Uebersetzer.
set(CMAKE_TRY_COMPILE_TARGET_TYPE STATIC_LIBRARY)

set(HART "--target=arm-linux-gnueabihf -march=armv7-a -mfloat-abi=hard -mfpu=neon -mthumb")
set(GEMEINSAM "${HART} --sysroot=${SYSROOT} -include ${SCHICHT}/../harmattan-schicht.h -isystem ${SCHICHT} -fno-strict-aliasing -D__STDC_FORMAT_MACROS -D__STDC_CONSTANT_MACROS")

# Dieselben libc++-Schalter wie im WebRTC-Bau. Weichen sie ab, passen die
# Symbole beim Binden nicht zusammen -- _LIBCPP_HARDENING_MODE steckt in
# jedem Vorlagennamen.
set(CXX_LIBCPP "-nostdinc++ -isystem ${WEBRTC_SRC}/third_party/libc++/src/include -isystem ${WEBRTC_SRC}/third_party/libc++abi/src/include -I${WEBRTC_SRC}/buildtools/third_party/libc++ -D_LIBCPP_HARDENING_MODE=_LIBCPP_HARDENING_MODE_EXTENSIVE -D_LIBCPP_DISABLE_VISIBILITY_ANNOTATIONS -D_LIBCXXABI_DISABLE_VISIBILITY_ANNOTATIONS")

set(CMAKE_C_FLAGS_INIT   "${GEMEINSAM}")
set(CMAKE_CXX_FLAGS_INIT "${GEMEINSAM} ${CXX_LIBCPP}")
set(CMAKE_EXE_LINKER_FLAGS_INIT "${HART} --sysroot=${SYSROOT} -fuse-ld=lld")

set(CMAKE_FIND_ROOT_PATH ${SYSROOT})
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
