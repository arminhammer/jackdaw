Output docs/vhs/persistence-demo.gif

Set Shell bash
Set FontSize 14
Set Width 1200
Set Height 800

Type "# Persistence Demo - First run will fail, second will resume"
Sleep 500ms
Enter
Sleep 500ms

Type "# First attempt - will fail at step 3"
Sleep 500ms
Enter
Sleep 500ms

Type 'jackdaw run examples/persistence/persistence.sw.yaml --persistence-provider redb --input '"'"'{"attempt": "1"}'"'"''
Sleep 500ms
Enter

Sleep 8s

Type "# Second attempt - will resume from saved state and complete"
Sleep 500ms
Enter
Sleep 500ms

Type 'jackdaw run examples/persistence/persistence.sw.yaml --persistence-provider redb --input '"'"'{"attempt": "2"}'"'"''
Sleep 500ms
Enter

Sleep 8s
