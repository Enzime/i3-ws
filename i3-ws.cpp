#define MAX_WORKSPACES_PER_OUTPUT 10

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


bool compare_rect(rect_t a, rect_t b) {
    if (a.x < b.x)
        return true;
    else if (a.x > b.x)
        return false;
    else if (a.width < b.width) // a.x == b.x at this point onwards
        return true;
    else if (a.width > b.width)
        return false;
    else if (a.y < b.y) // a.width == b.width
        return true;
    else if (a.y > b.y)
        return false;
    else if (a.height < b.height) // a.y == b.y
        return true;
    else if (a.height > b.height)
        return false;
    else // a.height == a.width (exactly the same)
        return true;
}


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


int strToInt(string input) {
    stringstream ss(input);
    int a;
    ss >> a;
    return a;
}


int main(int argc, char* argv[]) {
    // handle argv
    vector<string> args(argv + 1, argv + argc);

    bool valid = true;
    bool loop = false;
    bool create = false;
    bool init = false;
    bool shift = false;
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
        else if (*it == "--create")
            create = true;
        else if (*it == "--init")
            init = true;
        else if (*it == "--shift")
            shift = true;
        else
            valid = false;
    }

    if (!init && (!pos || pos > MAX_WORKSPACES_PER_OUTPUT) || init && pos)
        valid = false;

    // if no direction specified or invalid arguments
    if (!valid) {
        cout << "Usage: i3-ws [--init]" << endl;
        cout << "Usage: i3-ws [--ws] [--shift] {NUMBER}" << endl;
        cout << "Usage: i3-ws [--ws] [--shift] [--loop] (prev|next)" << endl;
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
        [](auto output) {
            return output->active; });

    active.resize(distance(active.begin(), it));

    sort(
        active.begin(),
        active.end(),
        [](out_ptr a, out_ptr b) {
            return compare_rect(a->rect, b->rect); });

    // get focused workspace
    auto workspaces = i3.get_workspaces();
    auto ws = find_if(
        workspaces.begin(),
        workspaces.end(),
        [](auto workspace) {
            return workspace->focused; });

    // get focused output
    auto current_output = find_if(
        active.begin(),
        active.end(),
        [ws](auto output) {
            return output->name == (*ws)->output; });

    vector<ws_ptr> current(workspaces.size());

    auto end = copy_if(
        workspaces.begin(),
        workspaces.end(),
        current.begin(),
        [current_output](auto workspace) {
            return workspace->output == (*current_output)->name; });

    if (init) {
        for (auto it = active.begin(); it != active.end(); ++it) {
            string ws_name;
            stringstream ss;
            ss << (distance(active.begin(), it) + 1) * 100 + 1;
            ss >> ws_name;
            cout << ws_name << endl;
            if ((*it)->current_workspace != ws_name) {
                if ((*it)->current_workspace != (*ws)->name)
                    i3.send_command("workspace " + (*it)->current_workspace);
                i3.send_command("workspace " + ws_name);
            }
        }

        return 0;
    }

    current.resize(distance(current.begin(), end));

    string command = "workspace ";

    if (shift)
        command = "move window to workspace ";

    // switch outputs
    if (active.size() > 1 && mode == "out") {
        auto current_output = find_if(
            active.begin(),
            active.end(),
            [ws](auto output) {
            return output->name == (*ws)->output; });

        auto active_index = distance(active.begin(), current_output);

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
        i3.send_command(command + n->current_workspace);

        // if moving a window to a new output, this will be on a visible
        // workspace, so focus that workspace after moving the window there
        if (shift)
            i3.send_command("workspace " + n->current_workspace);
    } else if (mode == "ws") {
    // switch (or create) workspasces :)
        auto cur = find(current.begin(), current.end(), *ws);
        cout << (*cur)->name << endl;

        int ws_num = strToInt((*cur)->name) % 100;

        vector<int> ws_nums;
        vector<int>::iterator new_ws;

        // ws_nums = {1, 2, 3 .. MAX_WORKSPACES_PER_OUTPUT}
        for (int i = 1; i <= MAX_WORKSPACES_PER_OUTPUT; i++)
            ws_nums.push_back(i);

        // -2 = next, -1 = prev, 0 = unset; 1-based index onwards
        if (pos == -2 || pos == -1) {
            // loop should only affect the edges (if you're on 1 or 10)
            // create should affect if you're going to jump to the next ws_num

            if (create) {
                new_ws = ws_nums.begin() + strToInt((*ws)->name) % 100 - 1;
            } else {
                ws_nums = {};
                for (auto &ws : current)
                    ws_nums.push_back(strToInt(ws->name) % 100);

                new_ws = ws_nums.begin() + distance(current.begin(), cur);
            }
        }

        // -2 = next, -1 = prev, 0 = unset; 1-based index onwards
        if (pos > 0) {
            // pos starts at 1, but items start from iter+0
            new_ws = ws_nums.begin() + pos - 1;
        }
        else if (pos == -1) {
            // if not first!
            if (new_ws != ws_nums.begin()) {
                new_ws -= 1;
            } else if (loop) {
            // this implies *new_ws == ws_nums[0]
                new_ws = ws_nums.end() - 1;
            } else {
                cout << "At the first workspace, use --loop" << endl;
                return 0;
            }
        }
        else if (pos == -2) {
            // if not last!
            if (new_ws != ws_nums.end() - 1) {
                new_ws += 1;
            } else if (loop) {
            // this implies *new_ws == ws_nums[-1]
                new_ws = ws_nums.begin();
            } else {
                cout << "At the last workspace, use --loop" << endl;
                return 0;
            }
        }

        string new_ws_name;
        stringstream new_wss;

        new_wss << ((distance(active.begin(), current_output) + 1) * 100
                    + *new_ws);
        new_wss >> new_ws_name;

        if (new_ws_name != (*cur)->name)
            i3.send_command(command + new_ws_name);
    }

    return 0;
}
