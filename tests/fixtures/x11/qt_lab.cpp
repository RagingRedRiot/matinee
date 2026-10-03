// A Qt 5 window run under X11, for named targeting through Xwayland.
#include <QApplication>
#include <QLabel>
#include <QLineEdit>
#include <QPushButton>
#include <QVBoxLayout>
#include <QWidget>

int main(int argc, char **argv) {
    QApplication app(argc, argv);
    QWidget window;
    window.setWindowTitle("Qt lab");
    auto *layout = new QVBoxLayout(&window);
    auto *entry = new QLineEdit;
    entry->setAccessibleName("Note");
    auto *button = new QPushButton("Save note");
    auto *status = new QLabel("not saved");
    layout->addWidget(entry);
    layout->addWidget(button);
    layout->addWidget(status);
    layout->addStretch();
    QObject::connect(button, &QPushButton::clicked, [=] {
        status->setText("saved: " + entry->text());
    });
    window.show();
    return app.exec();
}
