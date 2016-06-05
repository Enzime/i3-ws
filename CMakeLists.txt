cmake_minimum_required(VERSION 3.0)
project(i3-ws)

add_subdirectory(i3ipc++)

include_directories(
    ${I3IPCpp_INCLUDE_DIRS}
)
link_directories(
    ${I3IPCpp_LIBRARY_DIRS}
)

add_executable(i3-ws i3-ws.cpp)
target_link_libraries(i3-ws ${I3IPCpp_LIBRARIES})
