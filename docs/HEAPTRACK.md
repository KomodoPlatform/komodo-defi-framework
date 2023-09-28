# Memory profiling MM2 with heaptrack
1. Install dependencies required by heaptrack if they are not already installed on the system
* extra-cmake-modules
* Qt 5.2 or higher: Core, Widgets
* KDE Frameworks 5: CoreAddons, I18n, ItemModels, ThreadWeaver, ConfigWidgets, KIO, IconThemes

2. Install heaptrack on Ubuntu (18.04) or higher:
```
sudo apt install heaptrack heaptrack-gui
```

3. Use heaptrack to run MM2 binary and pass parameters as usual. An example for this would be:
```
heaptrack ./mm2 "{\"gui\":\"MM2GUI\",\"netid\":8762, \"userhome\":\"/${HOME#"/"}\", \"passphrase\":\"YOUR_PASSPHRASE_HERE\", \"rpc_password\":\"YOUR_PASSWORD_HERE\",\"i_am_seed\":true}" &
```
Running heaptrack like this writes a gzipped result file in the same folder the above command ran from. We can now take a look at using the next step.

4. After running MM2 for sometime we can visualize the memory profiling results using the below command. Note that ```heaptrack.mm2.xxxx.gz``` is the name of the file generated through the above command with numbers instead of xxxx
```
heaptrack_gui heaptrack.mm2.xxxx.gz
```