# AVL drum kits for the AVL Drums instrument (plugins/AvlDrums)
INSTALL(FILES
	Black_Pearl_4_LV2.sf2
	Red_Zeppelin_4_LV2.sf2
	Blonde_Bop_LV2.sf2
	Blonde_Bop_HR_LV2.sf2
	Buskmans_Holiday_LV2.sf2
	README
	SOURCE.txt
	DESTINATION "${LMMS_DATA_DIR}/avldrums")

INSTALL(FILES
	images/black_pearl.png images/black_pearl.map.png
	images/red_zeppelin.png images/red_zeppelin.map.png
	images/blonde_bop.png images/blonde_bop.map.png
	images/blonde_bop_hr.png
	images/buskman.png images/buskman.map.png
	DESTINATION "${LMMS_DATA_DIR}/avldrums/images")
