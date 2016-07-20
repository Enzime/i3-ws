#include <i3ipc++/ipc.hpp>
#include <cstring>
#include <iostream>
#include <algorithm>

int main(int argc, char* argv[]) {
    // handle argv
    std::vector<std::string> args(argv + 1, argv + argc);

    bool prev = false;
    bool valid = true;
    bool dirSet = false;
    bool loop = false;
    std::string mode = "out";

    for (auto it = args.begin(); it != args.end() && valid; ++it) {
        if (*it == "prev") {
            dirSet = true;
            prev = true;
        } else if (*it == "next")
            dirSet = true;
        else if (*it == "--ws")
            mode = "ws";
        else if (*it == "--loop")
            loop = true;
        else
            valid = false;
    }

    // if no direction specified or invalid arguments
    if (!valid || !dirSet) {
        std::cout << "Usage: i3-ws [--ws] [--loop] (prev|next)" << std::endl;
        return -1;
    }

    // connect to i3
    i3ipc::connection i3;

    // get active outputs
    auto outputs = i3.get_outputs();
    std::vector< std::shared_ptr<i3ipc::output_t> > active(outputs.size());

    auto it = std::copy_if(outputs.begin(), outputs.end(), active.begin(), [](auto output){ return output->active; });
    active.resize(std::distance(active.begin(), it));

    // get focused workspace
    auto workspaces = i3.get_workspaces();
    auto ws = std::find_if(workspaces.begin(), workspaces.end(), [](auto workspace){ return workspace->focused; });

    // get focused output
    auto out = std::find_if(active.begin(), active.end(), [ws](auto output){ return output->name == (*ws)->output; });

    std::vector< std::shared_ptr<i3ipc::workspace_t> > current(workspaces.size());

    auto end = std::copy_if(workspaces.begin(), workspaces.end(), current.begin(), [out](auto workspace){ return workspace->output == (*out)->name; });
    current.resize(std::distance(current.begin(), end));

    // switch outputs
    if (active.size() > 1 && mode == "out") {
        auto active_index = (std::distance(active.begin(), out) + (loop ? (prev ? 1 : -1) : 0) + active.size()) % active.size();
        auto n = active[active_index];
        std::cout << n->name << std::endl;
        i3.send_command("workspace " + n->current_workspace);
    } else if (current.size() > 1 || mode == "ws") { // switch workspaces
        auto cur = std::find(current.begin(), current.end(), *ws);
        std::cout << (*cur)->name << std::endl;

        auto current_index = (std::distance(active.begin(), out) + (loop ? (prev ? -1 : 1) : 0) + active.size()) % active.size();
        auto n = current[current_index];
        std::cout << n->name << std::endl;
        i3.send_command("workspace " + n->name);
    }

    return 0;
}
