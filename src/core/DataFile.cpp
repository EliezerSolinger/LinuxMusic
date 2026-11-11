/*
 * DataFile.cpp - implementation of class DataFile
 *
 * Copyright (c) 2004-2014 Tobias Doerffel <tobydox/at/users.sourceforge.net>
 * Copyright (c) 2012-2013 Paul Giblock    <p/at/pgiblock.net>
 *
 * This file is part of LMMS - https://lmms.io
 *
 * This program is free software; you can redistribute it and/or
 * modify it under the terms of the GNU General Public
 * License as published by the Free Software Foundation; either
 * version 2 of the License, or (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
 * General Public License for more details.
 *
 * You should have received a copy of the GNU General Public
 * License along with this program (see COPYING); if not, write to the
 * Free Software Foundation, Inc., 51 Franklin Street, Fifth Floor,
 * Boston, MA 02110-1301 USA.
 *
 */


#include "DataFile.h"

#include <algorithm>
#include <cmath>
#include <map>

#include <QDebug>
#include <QFile>
#include <QFileInfo>
#include <QDir>
#include <QMessageBox>
#include <QRegularExpression>
#include <QSaveFile>

#include "base64.h"
#include "ConfigManager.h"
#include "DeprecationHelper.h"
#include "Effect.h"
#include "embed.h"
#include "GuiApplication.h"
#include "LocaleHelper.h"
#include "Note.h"
#include "PluginFactory.h"
#include "ProjectVersion.h"
#include "SongEditor.h"
#include "TextFloat.h"
#include "Track.h"
#include "PathUtil.h"

#include "lmmsversion.h"

namespace lmms
{


static void findIds(const QDomElement& elem, QList<jo_id_t>& idList);


// QMap with the DOM elements that access file resources
const DataFile::ResourcesMap DataFile::ELEMENTS_WITH_RESOURCES = {
{ "sampleclip", {"src"} },
{ "audiofileprocessor", {"src"} },
};

namespace
{
	struct TypeDescStruct
	{
		DataFile::Type m_type;
		QString m_name;
	};

	const auto s_types = std::array{
		TypeDescStruct{ DataFile::Type::Unknown, "unknown" },
		TypeDescStruct{ DataFile::Type::SongProject, "song" },
		TypeDescStruct{ DataFile::Type::SongProjectTemplate, "songtemplate" },
		TypeDescStruct{ DataFile::Type::InstrumentTrackSettings, "instrumenttracksettings" },
		TypeDescStruct{ DataFile::Type::DragNDropData, "dnddata" },
		TypeDescStruct{ DataFile::Type::ClipboardData, "clipboard-data" },
		TypeDescStruct{ DataFile::Type::JournalData, "journaldata" },
		TypeDescStruct{ DataFile::Type::EffectSettings, "effectsettings" },
		TypeDescStruct{ DataFile::Type::MidiClip, "midiclip" }
	};
}




DataFile::DataFile( Type type ) :
	QDomDocument( "lmms-project" ),
	m_fileName(""),
	m_content(),
	m_head(),
	m_type( type ),
	m_fileVersion( FILE_VERSION )
{
	appendChild( createProcessingInstruction("xml", "version=\"1.0\""));
	QDomElement root = createElement( "lmms-project" );
	root.setAttribute( "version", m_fileVersion );
	root.setAttribute( "type", typeName( type ) );
	root.setAttribute( "creator", "LinuxMusic" );
	root.setAttribute( "creatorversion", LMMS_VERSION );
	root.setAttribute("creatorplatform", QSysInfo::kernelType());
	root.setAttribute("creatorplatformtype", QSysInfo::productType());
	appendChild( root );

	m_head = createElement( "head" );
	root.appendChild( m_head );

	m_content = createElement( typeName( type ) );
	root.appendChild( m_content );

}




DataFile::DataFile( const QString & _fileName ) :
	QDomDocument(),
	m_fileName(_fileName),
	m_content(),
	m_head(),
	m_fileVersion( FILE_VERSION )
{
	QFile inFile( _fileName );
	if( !inFile.open( QIODevice::ReadOnly ) )
	{
		if (gui::getGUI() != nullptr)
		{
			QMessageBox::critical( nullptr,
				gui::SongEditor::tr( "Could not open file" ),
				gui::SongEditor::tr( "Could not open file %1. You probably "
						"have no permissions to read this "
						"file.\n Please make sure to have at "
						"least read permissions to the file "
						"and try again." ).arg( _fileName ) );
		}

		return;
	}

	loadData( inFile.readAll(), _fileName );
}




DataFile::DataFile( const QByteArray & _data ) :
	QDomDocument(),
	m_fileName(""),
	m_content(),
	m_head(),
	m_fileVersion( FILE_VERSION )
{
	loadData( _data, "<internal data>" );
}





bool DataFile::validate( QString extension )
{
	switch( m_type )
	{
	case Type::SongProject:
		if( extension == "mmp" || extension == "mmpz" )
		{
			return true;
		}
		break;
	case Type::SongProjectTemplate:
		if(  extension == "mpt" )
		{
			return true;
		}
		break;
	case Type::InstrumentTrackSettings:
		if ( extension == "xpf" || extension == "xml" )
		{
			return true;
		}
		break;
	case Type::MidiClip:
		if (extension == "xpt" || extension == "xptz")
		{
			return true;
		}
		break;
	case Type::Unknown:
		if (! ( extension == "mmp" || extension == "mpt" || extension == "mmpz" ||
				extension == "xpf" || extension == "xml" ||
				( extension == "xiz" && ! getPluginFactory()->pluginSupportingExtension(extension).isNull()) ||
				extension == "sf2" || extension == "sf3" || extension == "pat" || extension == "mid" ||
				extension == "dll"
#ifdef LMMS_BUILD_LINUX
				|| extension == "so"
#endif
#ifdef LMMS_HAVE_LV2
				|| extension == "lv2"
#endif
				) )
		{
			return true;
		}
		if( extension == "wav" || extension == "ogg" || extension == "ds"
#ifdef LMMS_HAVE_SNDFILE_MP3
				|| extension == "mp3"
#endif
				)
		{
			return true;
		}
		break;
	default:
		return false;
	}
	return false;
}




QString DataFile::nameWithExtension( const QString & _fn ) const
{
	const QString extension = _fn.section( '.', -1 );

	switch( type() )
	{
		case Type::SongProject:
			if( extension != "mmp" &&
					extension != "mpt" &&
					extension != "mmpz" )
			{
				if( ConfigManager::inst()->value( "app",
						"nommpz" ).toInt() == 0 )
				{
					return _fn + ".mmpz";
				}
				return _fn + ".mmp";
			}
			break;
		case Type::SongProjectTemplate:
			if( extension != "mpt" )
			{
				return _fn + ".mpt";
			}
			break;
		case Type::InstrumentTrackSettings:
			if( extension != "xpf" )
			{
				return _fn + ".xpf";
			}
			break;
		default: ;
	}
	return _fn;
}




void DataFile::write( QTextStream & _strm )
{
	if( type() == Type::SongProject || type() == Type::SongProjectTemplate
					|| type() == Type::InstrumentTrackSettings )
	{
		cleanMetaNodes( documentElement() );
	}

	save(_strm, 2);
}




bool DataFile::writeFile(const QString& filename, bool withResources)
{
	// Small lambda function for displaying errors
	auto showError = [](QString title, QString body){
		if (gui::getGUI() != nullptr)
		{
			QMessageBox mb;
			mb.setWindowTitle(title);
			mb.setText(body);
			mb.setIcon(QMessageBox::Warning);
			mb.setStandardButtons(QMessageBox::Ok);
			mb.exec();
		}
		else
		{
			qWarning() << body;
		}
	};

	// If we are saving without resources, filename is just the file we are
	// saving to. If we are saving with resources (project bundle), filename
	// will be used (discarding extensions) to create a folder where the
	// bundle will be saved in

	QFileInfo fInfo(filename);

	const QString bundleDir = fInfo.path() + "/" + fInfo.fileName().section('.', 0, 0);
	const QString resourcesDir = bundleDir + "/resources";
	const QString fullName = withResources
		? nameWithExtension(bundleDir + "/" + fInfo.fileName())
		: nameWithExtension(filename);
	const QString fullNameTemp = fullName + ".new";
	const QString fullNameBak = fullName + ".bak";

	using gui::SongEditor;

	// If we are saving with resources, setup the bundle folder first
	if (withResources)
	{
		// First check if there's a bundle folder with the same name in
		// the path already. If so, warns user that we can't overwrite a
		// project bundle.
		if (QDir(bundleDir).exists())
		{
			showError(SongEditor::tr("Operation denied"),
				SongEditor::tr("A bundle folder with that name already exists on the "
				"selected path. Can't overwrite a project bundle. Please select a different "
				"name."));

			return false;
		}

		// Create bundle folder
		if (!QDir().mkdir(bundleDir))
		{
			showError(SongEditor::tr("Error"),
				SongEditor::tr("Couldn't create bundle folder."));
			return false;
		}

		// Create resources folder
		if (!QDir().mkdir(resourcesDir))
		{
			showError(SongEditor::tr("Error"),
				SongEditor::tr("Couldn't create resources folder."));
			return false;
		}

		// Copy resources to folder and update paths
		if (!copyResources(resourcesDir))
		{
			showError(SongEditor::tr("Error"),
				SongEditor::tr("Failed to copy resources."));
			return false;
		}
	}

	QSaveFile outfile(fullNameTemp);

	if (!outfile.open(QIODevice::WriteOnly | QIODevice::Truncate))
	{
		showError(SongEditor::tr("Could not write file"),
			SongEditor::tr("Could not open %1 for writing. You probably are not permitted to "
				"write to this file. Please make sure you have write-access to "
				"the file and try again.").arg(fullName));

		return false;
	}

	const QString extension = fullName.section('.', -1);
	if (extension == "mmpz" || extension == "xptz")
	{
		QString xml;
		QTextStream ts( &xml );
		write( ts );
		outfile.write( qCompress( xml.toUtf8() ) );
	}
	else
	{
		QTextStream ts( &outfile );
		write( ts );
	}

	if (!outfile.commit())
	{
		showError(SongEditor::tr("Could not write file"),
			SongEditor::tr("An unknown error has occurred and the file could not be saved."));
		return false;
	}

	if (ConfigManager::inst()->value("app", "disablebackup").toInt())
	{
		// remove current file
		QFile::remove(fullName);
	}
	else
	{
		// remove old backup file
		QFile::remove(fullNameBak);
		// move current file to backup file
		QFile::rename(fullName, fullNameBak);
	}
	// move temporary file to current file
	QFile::rename(fullNameTemp, fullName);

	return true;
}




bool DataFile::copyResources(const QString& resourcesDir)
{
	// List of filenames used so we can append a counter to any
	// repeating filenames
	std::list<QString> namesList;

	auto it = ELEMENTS_WITH_RESOURCES.begin();

	// Copy resources and manipulate the DataFile to have local paths to them
	while (it != ELEMENTS_WITH_RESOURCES.end())
	{
		QDomNodeList list = elementsByTagName(it->first);

		// Go through all elements with the tagname from our map
		for (int i = 0; !list.item(i).isNull(); ++i)
		{
			QDomElement el = list.item(i).toElement();

			auto res = it->second.begin();

			// Search for attributes that point to resources
			while (res != it->second.end())
			{
				// If the element has that attribute
				if (el.hasAttribute(*res))
				{
					// Get absolute path to resource
					bool error;
					auto attribute = el.attribute(*res);

					// Skip empty resources. In some cases, there might be an
					// alternative way of actually representing the data (e.g. data element for SampleTCO)
					if (attribute.isEmpty())
					{
						++res;
						continue;
					}

					QString resPath = PathUtil::toAbsolute(attribute, &error);
					// If we are running without the project loaded (from CLI), "local:" base
					// prefixes aren't converted, so we need to convert it ourselves
					if (error)
					{
						resPath = QFileInfo(m_fileName).path() + "/" + resPath.remove(0,
							PathUtil::basePrefix(PathUtil::Base::LocalDir).length());
					}

					// Check if we need to add a counter to the filename
					QString finalFileName = QFileInfo(resPath).fileName();
					QString extension = resPath.section('.', -1);
					int repeatedNames = 0;
					for (QString name : namesList)
					{
						if (finalFileName == name)
						{
							++repeatedNames;
						}
					}
					// Add the name to the list before modifying it
					namesList.push_back(finalFileName);
					if (repeatedNames)
					{
						// Remove the extension, add the counter and add the
						// extension again to get the final file name
						finalFileName.truncate(finalFileName.lastIndexOf('.'));
						finalFileName = finalFileName + "-" + QString::number(repeatedNames) + "." + extension;
					}

					// Final path is our resources dir + the new file name
					QString finalPath = resourcesDir + "/" + finalFileName;

					// Copy resource file to the resources folder
					if(!QFile::copy(resPath, finalPath))
					{
						qWarning("ERROR: Failed to copy resource");
						return false;
					}

					// Update attribute path to point to the bundle file
					QString newAtt = PathUtil::basePrefix(PathUtil::Base::LocalDir) + "resources/" + finalFileName;
					el.setAttribute(*res, newAtt);
				}
				++res;
			}
		}
		++it;
	}

	return true;
}




bool DataFile::hasLocalPlugins(QDomElement parent /* = QDomElement()*/, bool firstCall /* = true*/) const
{
	// If this is the first iteration of the recursion we use the root element
	if (firstCall) { parent = documentElement(); }

	auto children = parent.childNodes();
	for (int i = 0; i < children.size(); ++i)
	{
		QDomNode child = children.at(i);
		QDomElement childElement = child.toElement();

		bool skipNode = false;
		// Skip the nodes allowed to have "local:" attributes, but
		// still check its children
		for (const auto& element : ELEMENTS_WITH_RESOURCES)
		{
			if (childElement.tagName() == element.first)
			{
				skipNode = true;
				break;
			}
		}

		// Check if they have "local:" attribute (unless they are allowed to
		// and skipNode is true)
		if (!skipNode)
		{
			auto attributes = childElement.attributes();
			for (int i = 0; i < attributes.size(); ++i)
			{
				QDomNode attribute = attributes.item(i);
				QDomAttr attr = attribute.toAttr();
				if (attr.value().startsWith(PathUtil::basePrefix(PathUtil::Base::LocalDir),
					Qt::CaseInsensitive))
				{
					return true;
				}
			}
		}

		// Now we check the children of this node (recursively)
		// and if any return true we return true.
		if (hasLocalPlugins(childElement, false))
		{
			return true;
		}
	}

	// If we got here none of the nodes had the "local:" path.
	return false;
}




DataFile::Type DataFile::type( const QString& typeName )
{
	const auto it = std::find_if(s_types.begin(), s_types.end(),
		[&typeName](const TypeDescStruct& type) { return type.m_name == typeName; });
	if (it != s_types.end()) { return it->m_type; }

	// compat code
	if( typeName == "channelsettings" )
	{
		return Type::InstrumentTrackSettings;
	}

	if (typeName == "pattern")
	{
		return Type::MidiClip;
	}

	return Type::Unknown;
}




QString DataFile::typeName( Type type )
{
	return s_types[static_cast<std::size_t>(type)].m_name;
}




void DataFile::cleanMetaNodes( QDomElement _de )
{
	QDomNode node = _de.firstChild();
	while( !node.isNull() )
	{
		if( node.isElement() )
		{
			if( node.toElement().attribute( "metadata" ).toInt() )
			{
				QDomNode ns = node.nextSibling();
				_de.removeChild( node );
				node = ns;
				continue;
			}
			if( node.hasChildNodes() )
			{
				cleanMetaNodes( node.toElement() );
			}
		}
		node = node.nextSibling();
	}
}

void DataFile::loadData( const QByteArray & _data, const QString & _sourceFile )
{
	QString errorMsg;
	int line = -1, col = -1;
	if (!lmms::setContent(*this, _data, &errorMsg, &line, &col))
	{
		// parsing failed? then try to uncompress data
		QByteArray uncompressed = qUncompress( _data );
		if( !uncompressed.isEmpty() )
		{
			if (lmms::setContent(*this, uncompressed, &errorMsg, &line, &col))
			{
				line = col = -1;
			}
		}
		if( line >= 0 && col >= 0 )
		{
			using gui::SongEditor;

			qWarning() << "at line" << line << "column" << errorMsg;
			if (gui::getGUI() != nullptr)
			{
				QMessageBox::critical( nullptr,
					SongEditor::tr( "Error in file" ),
					SongEditor::tr( "The file %1 seems to contain "
							"errors and therefore can't be "
							"loaded." ).
								arg( _sourceFile ) );
			}

			return;
		}
	}

	QDomElement root = documentElement();
	m_type = type( root.attribute( "type" ) );
	m_head = root.elementsByTagName( "head" ).item( 0 ).toElement();

	bool success = false;
	m_fileVersion = root.attribute("version").toUInt(&success);
	if (!success) { m_fileVersion = 0; }

	if (root.hasAttribute("creatorversion"))
	{
		using gui::SongEditor;

		// compareType defaults to All, so it doesn't have to be set here
		ProjectVersion createdWith = root.attribute("creatorversion");
		ProjectVersion openedWith = LMMS_VERSION;

		if (createdWith.setCompareType(ProjectVersion::CompareType::Minor)
		 !=  openedWith.setCompareType(ProjectVersion::CompareType::Minor)
		 && gui::getGUI() != nullptr && root.attribute("type") == "song"
		){
			auto projectType = _sourceFile.endsWith(".mpt") ?
				SongEditor::tr("template") : SongEditor::tr("project");

			gui::TextFloat::displayMessage(
				SongEditor::tr("Version difference"),
				SongEditor::tr("This %1 was created with LinuxMusic %2")
				.arg(projectType).arg(createdWith.getVersion()),
				embed::getIconPixmap("whatsthis", 24, 24),
				2500
			);
		}
	}

	// LinuxMusic does not convert files from older LMMS versions; load them as they are
	if (m_fileVersion < FILE_VERSION)
	{
		qWarning() << "File" << _sourceFile << "uses an old file format (version" << m_fileVersion
			<< ") and may not load correctly";
		if (gui::getGUI() != nullptr && root.attribute("type") == "song")
		{
			gui::TextFloat::displayMessage(
				gui::SongEditor::tr("Old file format"),
				gui::SongEditor::tr("This file was made with an older version and may not load correctly."),
				embed::getIconPixmap("whatsthis", 24, 24),
				4000
			);
		}
	}

	m_content = root.elementsByTagName(typeName(m_type)).item(0).toElement();
}


void findIds(const QDomElement& elem, QList<jo_id_t>& idList)
{
	if(elem.hasAttribute("id"))
	{
		idList.append(elem.attribute("id").toInt());
	}
	QDomElement child = elem.firstChildElement();
	while(!child.isNull())
	{
		findIds(child, idList);
		child = child.nextSiblingElement();
	}
}

} // namespace lmms
