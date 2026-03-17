from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, cast

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

T = TypeVar("T", bound="ApiCreateContentRequest")


@_attrs_define
class ApiCreateContentRequest:
    draft: bool
    page_content: str
    page_type: str
    slug: str
    title: str
    published_at: None | str | Unset = UNSET
    tags: list[str] | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)

    def to_dict(self) -> dict[str, Any]:
        draft = self.draft

        page_content = self.page_content

        page_type = self.page_type

        slug = self.slug

        title = self.title

        published_at: None | str | Unset
        if isinstance(self.published_at, Unset):
            published_at = UNSET
        else:
            published_at = self.published_at

        tags: list[str] | Unset = UNSET
        if not isinstance(self.tags, Unset):
            tags = self.tags

        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update(
            {
                "draft": draft,
                "page_content": page_content,
                "page_type": page_type,
                "slug": slug,
                "title": title,
            }
        )
        if published_at is not UNSET:
            field_dict["published_at"] = published_at
        if tags is not UNSET:
            field_dict["tags"] = tags

        return field_dict

    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        draft = d.pop("draft")

        page_content = d.pop("page_content")

        page_type = d.pop("page_type")

        slug = d.pop("slug")

        title = d.pop("title")

        def _parse_published_at(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        published_at = _parse_published_at(d.pop("published_at", UNSET))

        tags = cast(list[str], d.pop("tags", UNSET))

        api_create_content_request = cls(
            draft=draft,
            page_content=page_content,
            page_type=page_type,
            slug=slug,
            title=title,
            published_at=published_at,
            tags=tags,
        )

        api_create_content_request.additional_properties = d
        return api_create_content_request

    @property
    def additional_keys(self) -> list[str]:
        return list(self.additional_properties.keys())

    def __getitem__(self, key: str) -> Any:
        return self.additional_properties[key]

    def __setitem__(self, key: str, value: Any) -> None:
        self.additional_properties[key] = value

    def __delitem__(self, key: str) -> None:
        del self.additional_properties[key]

    def __contains__(self, key: str) -> bool:
        return key in self.additional_properties
