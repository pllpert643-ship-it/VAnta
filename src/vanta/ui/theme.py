APP_STYLESHEET = """
QWidget {
    color: #E7E9F2;
    font-family: "Segoe UI";
    font-size: 13px;
}
QMainWindow, QWidget#root {
    background: #090A0F;
}
QFrame#topBar {
    background: #0D0F16;
    border-bottom: 1px solid #222633;
}
QLabel#brand {
    color: #FFFFFF;
    font-size: 25px;
    font-weight: 700;
    letter-spacing: 2px;
}
QLabel#brandMark {
    color: #9B7CFF;
    font-size: 28px;
    font-weight: 800;
}
QLabel#muted, QLabel.muted {
    color: #858A9B;
}
QLabel#sectionTitle {
    color: #F7F7FB;
    font-size: 17px;
    font-weight: 650;
}
QLabel#overline {
    color: #858A9B;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 1px;
}
QFrame#card {
    background: #11131B;
    border: 1px solid #222633;
    border-radius: 12px;
}
QFrame#metricCard {
    background: #11131B;
    border: 1px solid #222633;
    border-radius: 11px;
}
QLabel#metricValue {
    color: #FFFFFF;
    font-size: 25px;
    font-weight: 700;
}
QLabel#metricAccent {
    color: #A98DFF;
    font-size: 25px;
    font-weight: 700;
}
QLabel#livePill {
    color: #91F3C3;
    background: #10231C;
    border: 1px solid #1E5B42;
    border-radius: 12px;
    padding: 5px 10px;
    font-size: 11px;
    font-weight: 700;
}
QPushButton {
    background: #1B1E29;
    border: 1px solid #303544;
    border-radius: 8px;
    padding: 8px 13px;
    font-weight: 600;
}
QPushButton:hover {
    background: #252938;
    border-color: #484F63;
}
QPushButton:pressed {
    background: #171923;
}
QPushButton:focus {
    border: 1px solid #A98DFF;
}
QPushButton#primaryButton {
    color: #090A0F;
    background: #A98DFF;
    border: 1px solid #BCA7FF;
    padding: 10px 16px;
    font-weight: 750;
}
QPushButton#primaryButton:hover {
    background: #B9A3FF;
}
QPushButton#dangerButton {
    color: #FF9DA7;
    background: #24151A;
    border-color: #572933;
}
QLineEdit, QComboBox, QSpinBox {
    background: #0C0E14;
    border: 1px solid #2A2F3D;
    border-radius: 7px;
    padding: 8px 10px;
    selection-background-color: #6E55C7;
}
QLineEdit:focus, QComboBox:focus, QSpinBox:focus {
    border-color: #9B7CFF;
}
QComboBox::drop-down {
    border: none;
    width: 24px;
}
QComboBox QAbstractItemView {
    background: #151822;
    border: 1px solid #303544;
    selection-background-color: #302852;
    outline: none;
}
QTableWidget {
    background: transparent;
    alternate-background-color: #0E1017;
    border: none;
    gridline-color: transparent;
    selection-background-color: #25213A;
    selection-color: #FFFFFF;
    outline: none;
}
QHeaderView::section {
    color: #858A9B;
    background: #11131B;
    border: none;
    border-bottom: 1px solid #252936;
    padding: 9px 8px;
    font-size: 10px;
    font-weight: 700;
}
QTableWidget::item {
    border-bottom: 1px solid #1B1E29;
    padding: 8px;
}
QListWidget {
    background: transparent;
    border: none;
    outline: none;
}
QListWidget::item {
    border-bottom: 1px solid #1E212C;
    padding: 9px 4px;
}
QListWidget::item:selected {
    background: #1A1C27;
}
QScrollBar:vertical {
    background: transparent;
    width: 8px;
    margin: 0;
}
QScrollBar::handle:vertical {
    background: #333746;
    border-radius: 4px;
    min-height: 30px;
}
QScrollBar::add-line:vertical, QScrollBar::sub-line:vertical {
    height: 0;
}
QToolTip {
    color: #FFFFFF;
    background: #20232E;
    border: 1px solid #414657;
    padding: 5px;
}
"""

APP_STYLESHEET += """
QTabWidget::pane { border: 1px solid #222633; background: #090A0F; }
QTabBar::tab { background: #11131B; color: #9297A8; padding: 11px 16px; border-bottom: 2px solid transparent; }
QTabBar::tab:selected { background: #211C38; color: #C4B4FF; border-bottom: 2px solid #A98DFF; }
QTabBar::tab:hover { background: #25213A; }
QMenu { background: #151822; color: #E7E9F2; border: 1px solid #303544; padding: 5px; }
QMenu::item { padding: 7px 22px; }
QMenu::item:selected { background: #302852; }
QMenu::item:disabled { color: #606575; }
QCheckBox { padding: 5px; }
QDialog { background: #11131B; }
QPlainTextEdit { background: #0C0E14; color: #E7E9F2; }
"""
