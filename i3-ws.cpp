#include <i3ipc++/ipc.hpp>
#include <algorithm>
#include <cstring>
#include <iostream>
#include <sstream>

using namespace std;
using i3ipc::output_t;
using i3ipc::workspace_t;
using i3ipc::container_t;
using i3ipc::rect_t;

using con_ptr = shared_ptr<container_t>;
using out_ptr = shared_ptr<output_t>;
using ws_ptr = shared_ptr<workspace_t>;


bool isPosInt(string input) {
    string chars = "0123456789";
    for (auto i : input) {
        bool digit = false;
        for (auto c : chars)
            if (i == c)
                digit = true;
        if (!digit)
            return false;
    }

    return true;
}


int main(int argc, char* argv[]) {
    // handle argv
    vector<string> args(argv + 1, argv + argc);

    bool valid = true;
    bool loop = false;
    int pos = 0; // -2 = next, -1 = prev, 0 = unset; 1-based index onwards
    string mode = "out";

    for (auto it = args.begin(); it != args.end() && valid; ++it) {
        if (*it == "prev" && !pos) {
            pos = -1;
        } else if (*it == "next" && !pos)
            pos = -2;
        else if (isPosInt(*it) && !pos) {
            stringstream sstream(*it);
            sstream >> pos;
        } else if (*it == "--ws")
            mode = "ws";
        else if (*it == "--loop")
            loop = true;
        else
            valid = false;
    }

    // if no direction specified or invalid arguments
    if (!valid || !pos) {
        cout << "Usage: i3-ws [--ws] [--loop] (prev|next|{NUMBER})";
        cout << endl;
        return -1;
    }

    // connect to i3
    i3ipc::connection i3;

    // get active outputs
    auto outputs = i3.get_outputs();
    vector<out_ptr> active(outputs.size());

    auto it = copy_if(
        outputs.begin(),
        outputs.end(),
        active.begin(),
        [](auto output){ return output->active; }
    );

    active.resize(distance(active.begin(), it));

    // get focused workspace
    auto workspaces = i3.get_workspaces();
    auto ws = find_if(
        workspaces.begin(),
        workspaces.end(),
        [](auto workspace){ return workspace->focused; }
    );

    // get focused output
    auto current_ouput = find_if(
        active.begin(),
        active.end(),
        [ws](auto output) {
            return output->name == (*ws)->output; });

    vector<ws_ptr> current(workspaces.size());

    auto end = copy_if(
        workspaces.begin(),
        workspaces.end(),
        current.begin(),
        [current_ouput](auto workspace) {
            return workspace->output == (*current_ouput)->name; });

    current.resize(distance(current.begin(), end));

    // switch outputs
    if (active.size() > 1 && mode == "out") {
        sort(active.begin(), active.end(), [](auto a, auto b) {
            return a->rect.x < b->rect.x;
        });

        auto current_ouput = find_if(
            active.begin(),
            active.end(),
            [ws](auto output){ return output->name == (*ws)->output; }
        );

        auto active_index = distance(active.begin(), current_ouput);

        if (pos > 0 && pos - 1 != active_index)
            active_index = pos - 1;
        else if (pos == -1)
            active_index = active_index - 1;
        else if (pos == -2)
            active_index = active_index + 1;
        else
            return 0;

        if (loop) {
            active_index = (active_index + active.size()) % active.size();
        } else if (active_index < 0 || active_index >= active.size()) {
            return 0;
        }

        cout << active_index << endl;

        auto n = active[active_index];
        cout << n->name << " " << n->current_workspace << endl;
        i3.send_command("workspace " + n->current_workspace);
    } else if (current.size() > 1 && mode == "ws") { // switch workspaces
        auto cur = find(current.begin(), current.end(), *ws);
        cout << (*cur)->name << endl;

        auto current_index = distance(current.begin(), cur);

        if (pos > 0 && pos - 1 != current_index)
            current_index = pos - 1;
        else if (pos == -1)
            current_index = current_index - 1;
        else if (pos == -2)
            current_index = current_index + 1;
        else
            return 0;

        if (loop) {
            current_index = (current_index + current.size()) % current.size();
        } else if (current_index < 0 || current_index >= current.size()) {
            return 0;
        }

        auto n = current[current_index];
        cout << n->name << endl;
        i3.send_command("workspace " + n->name);
    }

    return 0;
}
