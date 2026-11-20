# caption: <b>Pipe</b> data in and out with -

Type "cat server.log | vaqum compress - > server.log.vqm"
Sleep 300ms
Enter
Sleep 2000ms
Type "vaqum decompress server.log.vqm -o - | tail -n 3"
Sleep 300ms
Enter
Sleep 3500ms
