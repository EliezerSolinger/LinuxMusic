#!/usr/bin/env bash
# Checks that the directories in tools/check/qt-free-dirs.txt use no Qt: no Qt header included, no Qt type or macro
# (Q_OBJECT, QString, signals, slots, emit...). Fails listing what it found. Usage: qt-free.sh [repository root]
root=${1:-$(cd "$(dirname "$0")/../.." && pwd)}
list="$root/tools/check/qt-free-dirs.txt"
status=0
checked=0
while IFS= read -r dir; do
	case "$dir" in ''|'#'*) continue ;; esac
	if [ ! -d "$root/$dir" ]; then echo "qt-free: $dir does not exist"; status=1; continue; fi
	checked=$((checked + 1))
	# Qt's headers (<QtCore/...>, <QString>, "qglobal.h"...), and what only Qt's moc or types give
	found=$(grep -rnE --include='*.h' --include='*.hpp' --include='*.cpp' --include='*.cc' \
		'#[[:space:]]*include[[:space:]]*[<"](Qt[A-Za-z]*/|Q[A-Z][A-Za-z]+(\.h)?[>"]|q[a-z]+\.h[>"])|\bQ_OBJECT\b|\bQ_GADGET\b|\bQ_PROPERTY\b|\bQ(String|Object|Vector|List|Map|Hash|Variant|ByteArray|Timer|Thread|Mutex|DomElement|DomDocument|Pointer)\b|^[[:space:]]*(signals|public slots|private slots|protected slots)[[:space:]]*:|\bemit[[:space:]]' \
		"$root/$dir")
	if [ -n "$found" ]; then echo "qt-free: Qt used in $dir:"; echo "$found"; status=1; fi
done < "$list"
[ $status -eq 0 ] && echo "qt-free: $checked directories, no Qt"
exit $status
