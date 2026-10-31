#!/usr/bin/env python3
import gi
import os

gi.require_version("Gtk", "3.0")
from gi.repository import Gtk

class FileChooserWindow(Gtk.Window):

    def __init__(self):
        super().__init__()
        # Set up Glade File 
        self.builder = Gtk.Builder()     
        self.glade_file = __file__.rpartition('.')[0] +'.glade'
        self.builder.add_from_file(self.glade_file)

        class Handlers:
            source_file_selected = ''
            source_folder_selected = ''
            dest_folder_selected = '~/Pictures'
            convert_single_file = False
            source_label = self.builder.get_object("source_label")
            dest_label = self.builder.get_object("dest_label")
            convert_button = self.builder.get_object("convert_button")
            command_output_label = self.builder.get_object("command_output_label")
            
            def on_source_file_button_clicked(self, widget):
                dialog = Gtk.FileChooserDialog(
                    title="Choose a CR3 image to Convert to DNG", 
                    parent=None, 
                    action=Gtk.FileChooserAction.OPEN
                )
                dialog.set_create_folders(True)
                dialog.add_buttons(
                    Gtk.STOCK_CANCEL,
                    Gtk.ResponseType.CANCEL,
                    Gtk.STOCK_OPEN,
                    Gtk.ResponseType.OK,
                )
                
                self.add_filters(dialog)
                response = dialog.run()
                if response == Gtk.ResponseType.OK:
                    self.source_file_selected = dialog.get_filename()
                    self.convert_button.set_sensitive(True)
                    self.convert_single_file = True
                    print("Source file selected: " + self.source_file_selected)
                    self.source_label.set_text('Source: ' + self.source_file_selected)
                dialog.destroy()

            def on_source_folder_button_clicked(self, widget):
                dialog = Gtk.FileChooserDialog(
                    title="Choose a Source Folder of CR3 Images to Transfer", 
                    parent=None, 
                    action=Gtk.FileChooserAction.SELECT_FOLDER
                )
                dialog.add_buttons(
                    Gtk.STOCK_CANCEL,
                    Gtk.ResponseType.CANCEL,
                    Gtk.STOCK_OPEN,
                    Gtk.ResponseType.OK,
                )
                response = dialog.run()
                if response == Gtk.ResponseType.OK:
                    self.source_folder_selected = dialog.get_filename()
                    self.convert_button.set_sensitive(True)
                    self.convert_single_file = False
                    print("Source folder selected: " + self.source_folder_selected)
                    self.source_label.set_text(self.source_folder_selected)
                dialog.destroy()

            def add_filters(self, dialog):
                filter_cr3 = Gtk.FileFilter()
                filter_cr3.set_name("CR3")
                filter_cr3.add_pattern("*.CR3")
                filter_cr3.add_mime_type('image/x-canon-cr3')
                dialog.add_filter(filter_cr3)

                filter_any = Gtk.FileFilter()
                filter_any.set_name("Any files")
                filter_any.add_pattern("*")
                dialog.add_filter(filter_any)

            def on_dest_folder_button_clicked(self, widget):
                dialog = Gtk.FileChooserDialog(
                    title="Please choose a folder",
                    parent=None,
                    action=Gtk.FileChooserAction.CREATE_FOLDER
                )
                dialog.add_buttons(
                    Gtk.STOCK_CANCEL, Gtk.ResponseType.CANCEL, "Select", Gtk.ResponseType.OK
                )
                dialog.set_default_size(800, 400)
                response = dialog.run()
                if response == Gtk.ResponseType.OK:
                    self.dest_folder_selected = dialog.get_filename()
                    print("Folder selected: " + self.dest_folder_selected)
                    self.dest_label.set_text(self.dest_folder_selected)
                dialog.destroy()
                
            def on_convert_button_clicked(self, widget):
                if self.convert_single_file:
                    command = "dnglab convert " + self.source_file_selected + \
                                ' ' + self.dest_folder_selected + "/" + \
                                str(self.source_file_selected.split('.')[0].split('/')[-1]) + \
                                '.DNG'
                else :
                    command = "dnglab convert " + self.source_folder_selected + \
                                ' ' + self.dest_folder_selected
                #self.convert_button.set_sensitive(False)  # Need to set sensitive after conversion?ls
                print(command)
                os.system("gnome-terminal -- bash -c '" + command +"; $SHELL'")
               
        self.builder.connect_signals(Handlers())
        self.file_chooser_window = self.builder.get_object("file_chooser_window")
        self.file_chooser_window.connect("destroy", Gtk.main_quit)
        self.file_chooser_window.show()
                  
    def main(self):
        Gtk.main()            

if __name__ == "__main__":
    application = FileChooserWindow()
    application.main()