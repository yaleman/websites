from http import HTTPStatus
from typing import Any
from urllib.parse import quote
from uuid import UUID

import httpx

from ... import errors
from ...client import AuthenticatedClient, Client
from ...models.api_content_response import ApiContentResponse
from ...models.api_error_response import ApiErrorResponse
from ...types import Response


def _get_kwargs(
    site_id: UUID,
    content_id: UUID,
) -> dict[str, Any]:

    _kwargs: dict[str, Any] = {
        "method": "get",
        "url": "/api/site/{site_id}/content/{content_id}".format(
            site_id=quote(str(site_id), safe=""),
            content_id=quote(str(content_id), safe=""),
        ),
    }

    return _kwargs


def _parse_response(
    *, client: AuthenticatedClient | Client, response: httpx.Response
) -> ApiContentResponse | ApiErrorResponse | None:
    if response.status_code == 200:
        response_200 = ApiContentResponse.from_dict(response.json())

        return response_200

    if response.status_code == 401:
        response_401 = ApiErrorResponse.from_dict(response.json())

        return response_401

    if response.status_code == 403:
        response_403 = ApiErrorResponse.from_dict(response.json())

        return response_403

    if response.status_code == 404:
        response_404 = ApiErrorResponse.from_dict(response.json())

        return response_404

    if response.status_code == 500:
        response_500 = ApiErrorResponse.from_dict(response.json())

        return response_500

    if client.raise_on_unexpected_status:
        raise errors.UnexpectedStatus(response.status_code, response.content)
    else:
        return None


def _build_response(
    *, client: AuthenticatedClient | Client, response: httpx.Response
) -> Response[ApiContentResponse | ApiErrorResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    site_id: UUID,
    content_id: UUID,
    *,
    client: AuthenticatedClient,
) -> Response[ApiContentResponse | ApiErrorResponse]:
    """
    Args:
        site_id (UUID):
        content_id (UUID):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ApiContentResponse | ApiErrorResponse]
    """

    kwargs = _get_kwargs(
        site_id=site_id,
        content_id=content_id,
    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)


def sync(
    site_id: UUID,
    content_id: UUID,
    *,
    client: AuthenticatedClient,
) -> ApiContentResponse | ApiErrorResponse | None:
    """
    Args:
        site_id (UUID):
        content_id (UUID):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ApiContentResponse | ApiErrorResponse
    """

    return sync_detailed(
        site_id=site_id,
        content_id=content_id,
        client=client,
    ).parsed


async def asyncio_detailed(
    site_id: UUID,
    content_id: UUID,
    *,
    client: AuthenticatedClient,
) -> Response[ApiContentResponse | ApiErrorResponse]:
    """
    Args:
        site_id (UUID):
        content_id (UUID):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ApiContentResponse | ApiErrorResponse]
    """

    kwargs = _get_kwargs(
        site_id=site_id,
        content_id=content_id,
    )

    response = await client.get_async_httpx_client().request(**kwargs)

    return _build_response(client=client, response=response)


async def asyncio(
    site_id: UUID,
    content_id: UUID,
    *,
    client: AuthenticatedClient,
) -> ApiContentResponse | ApiErrorResponse | None:
    """
    Args:
        site_id (UUID):
        content_id (UUID):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ApiContentResponse | ApiErrorResponse
    """

    return (
        await asyncio_detailed(
            site_id=site_id,
            content_id=content_id,
            client=client,
        )
    ).parsed
