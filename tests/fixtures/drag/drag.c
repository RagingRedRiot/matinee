// A GTK 4 window for testing drag steps: a label to drag onto a button,
// and a slider whose value is shown rounded to tens.
#include <gtk/gtk.h>
#include <math.h>

static GtkWidget *drop_status;
static GtkWidget *volume_status;

static GdkContentProvider *prepare(GtkDragSource *source, double x, double y, gpointer data) {
    return gdk_content_provider_new_typed(G_TYPE_STRING, "token");
}

static gboolean dropped(GtkDropTarget *target, const GValue *value, double x, double y, gpointer data) {
    char *text = g_strdup_printf("dropped: %s", g_value_get_string(value));
    gtk_label_set_text(GTK_LABEL(drop_status), text);
    g_free(text);
    return TRUE;
}

static void volume_changed(GtkRange *range, gpointer data) {
    int tens = (int)lround(gtk_range_get_value(range) / 10.0) * 10;
    char *text = g_strdup_printf("volume: %d", tens);
    gtk_label_set_text(GTK_LABEL(volume_status), text);
    g_free(text);
}

static void activate(GtkApplication *app, gpointer data) {
    GtkWidget *window = gtk_application_window_new(app);
    gtk_window_set_title(GTK_WINDOW(window), "Drag lab");
    GtkWidget *box = gtk_box_new(GTK_ORIENTATION_VERTICAL, 24);
    gtk_widget_set_margin_start(box, 40);
    gtk_widget_set_margin_end(box, 40);
    gtk_widget_set_margin_top(box, 40);

    GtkWidget *row = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 120);
    GtkWidget *source = gtk_label_new("Drag me");
    GtkDragSource *drag = gtk_drag_source_new();
    gtk_drag_source_set_actions(drag, GDK_ACTION_COPY);
    g_signal_connect(drag, "prepare", G_CALLBACK(prepare), NULL);
    gtk_widget_add_controller(source, GTK_EVENT_CONTROLLER(drag));
    GtkWidget *target = gtk_button_new_with_label("Drop here");
    GtkDropTarget *drop = gtk_drop_target_new(G_TYPE_STRING, GDK_ACTION_COPY);
    g_signal_connect(drop, "drop", G_CALLBACK(dropped), NULL);
    gtk_widget_add_controller(target, GTK_EVENT_CONTROLLER(drop));
    gtk_box_append(GTK_BOX(row), source);
    gtk_box_append(GTK_BOX(row), target);
    gtk_box_append(GTK_BOX(box), row);
    drop_status = gtk_label_new("nothing dropped");
    gtk_box_append(GTK_BOX(box), drop_status);

    GtkWidget *scale = gtk_scale_new_with_range(GTK_ORIENTATION_HORIZONTAL, 0, 100, 1);
    gtk_range_set_value(GTK_RANGE(scale), 20);
    gtk_accessible_update_property(GTK_ACCESSIBLE(scale), GTK_ACCESSIBLE_PROPERTY_LABEL, "Volume", -1);
    g_signal_connect(scale, "value-changed", G_CALLBACK(volume_changed), NULL);
    gtk_box_append(GTK_BOX(box), scale);
    volume_status = gtk_label_new("volume: 20");
    gtk_box_append(GTK_BOX(box), volume_status);

    gtk_window_set_child(GTK_WINDOW(window), box);
    gtk_window_present(GTK_WINDOW(window));
}

int main(int argc, char **argv) {
    GtkApplication *app = gtk_application_new("dev.matinee.DragLab", G_APPLICATION_NON_UNIQUE);
    g_signal_connect(app, "activate", G_CALLBACK(activate), NULL);
    return g_application_run(G_APPLICATION(app), argc, argv);
}
