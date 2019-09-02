#/bin/bash

for f in `find . -iname "*.snap.new"`; do
  echo ""

  read -p "Rename $f to ${f/.snap.new/.snap}? [yN] " answer

  case $answer in
    [Yy]*) mv "$f" "${f/.snap.new/.snap}" && echo "Done.";;
    *) echo "Not renaming $f";;
  esac
done